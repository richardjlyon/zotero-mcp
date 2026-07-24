#!/usr/bin/env python3
"""Generate test PDFs for the pdf-extraction tests.

Requires:  pip install pikepdf pillow reportlab

Run from anywhere; produces files next to this script.
"""
import zlib
from pathlib import Path

try:
    import pikepdf
except ImportError as e:
    raise SystemExit(
        "Install pikepdf first:  pip install pikepdf  (or uv pip install pikepdf)"
    ) from e


HERE = Path(__file__).resolve().parent


def make_hello() -> None:
    """A minimal valid PDF containing 'Hello fallback world.' so pdftotext can extract text."""
    pdf = pikepdf.Pdf.new()
    pdf.add_blank_page(page_size=(612, 792))
    page = pdf.pages[0]
    # Embed a Type 1 (PostScript) Helvetica font reference.
    font = pikepdf.Dictionary(
        Type=pikepdf.Name("/Font"),
        Subtype=pikepdf.Name("/Type1"),
        BaseFont=pikepdf.Name("/Helvetica"),
    )
    page.Resources = pikepdf.Dictionary(
        Font=pikepdf.Dictionary(F1=pdf.make_indirect(font)),
    )
    content = b"BT /F1 14 Tf 72 720 Td (Hello fallback world.) Tj ET"
    page.Contents = pdf.make_stream(content)
    out = HERE / "hello.pdf"
    pdf.save(out)
    print(f"Wrote {out} ({out.stat().st_size} bytes)")


# Note: make_type4 attempted to synthesize a minimal PDF that triggers
# pdf-extract's "unhandled function type 4" panic. Hand-authored fixtures
# do not reliably reproduce the panic — the real-world trigger appears to
# require a font CMap path that's difficult to synthesize. Coverage of
# this case is provided by:
# - Orchestrator unit tests (core::pdf::orchestrator_tests) with stub engines.
# - Direct repro against the offending PDF at smoke-test time.
#
# Approach tried: Type 3 font whose glyph CharProc invokes an axial shading
# whose /Function is a Type 4 PostScript-calculator stream. pdf-extract
# processed the file without error (returned Ok("\nA")), so the panic path
# was not reached via shading — it requires a deeper font-CMap decode route.


def make_scanned() -> None:
    """An image-only (scanned-style) PDF with NO text layer.

    The page is a single greyscale raster of the sentence below, so text
    extraction yields nothing until an OCR pass (ocrmypdf) adds a text layer.
    """
    try:
        from PIL import Image, ImageDraw, ImageFont
    except ImportError as e:
        raise SystemExit(
            "Install pillow first:  pip install pillow  (or uv pip install pillow)"
        ) from e

    text = "Scanned quarterly report recovered by optical character recognition."
    # 612x792pt page rendered at 2x (144 dpi equivalent) for OCR accuracy.
    img = Image.new("L", (1224, 1584), 255)
    draw = ImageDraw.Draw(img)
    font = None
    for candidate in (
        "/System/Library/Fonts/Helvetica.ttc",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ):
        try:
            font = ImageFont.truetype(candidate, 40)
            break
        except OSError:
            continue
    if font is None:
        font = ImageFont.load_default(size=40)
    # Two lines so tesseract sees a block of prose, not a stray caption.
    words = text.split()
    draw.text((100, 300), " ".join(words[:4]), font=font, fill=0)
    draw.text((100, 380), " ".join(words[4:]), font=font, fill=0)

    pdf = pikepdf.Pdf.new()
    pdf.add_blank_page(page_size=(612, 792))
    page = pdf.pages[0]
    image = pikepdf.Stream(pdf, zlib.compress(img.tobytes()))
    image.Type = pikepdf.Name("/XObject")
    image.Subtype = pikepdf.Name("/Image")
    image.Width, image.Height = img.size
    image.ColorSpace = pikepdf.Name("/DeviceGray")
    image.BitsPerComponent = 8
    image.Filter = pikepdf.Name("/FlateDecode")
    page.Resources = pikepdf.Dictionary(
        XObject=pikepdf.Dictionary(Im0=pdf.make_indirect(image)),
    )
    page.Contents = pdf.make_stream(b"q 612 0 0 792 0 0 cm /Im0 Do Q")
    out = HERE / "scanned.pdf"
    pdf.save(out)
    print(f"Wrote {out} ({out.stat().st_size} bytes)")


def _reportlab():
    try:
        from reportlab.lib.pagesizes import letter
        from reportlab.pdfgen import canvas

        return letter, canvas
    except ImportError as e:
        raise SystemExit(
            "Install reportlab first:  pip install reportlab  (or uv pip install reportlab)"
        ) from e


def make_equation() -> None:
    """A one-page paper-like PDF with prose and a display equation.

    The equation is the quadratic formula drawn with a real fraction bar and
    a radical, so Docling's layout model classifies the region as a formula
    and (with do_formula_enrichment=true) decodes it to LaTeX.
    """
    letter, canvas = _reportlab()
    out = HERE / "equation.pdf"
    c = canvas.Canvas(str(out), pagesize=letter)
    width, _height = letter

    c.setFont("Helvetica-Bold", 16)
    c.drawCentredString(width / 2, 730, "On the Roots of Quadratic Polynomials")
    c.setFont("Helvetica", 11)
    text = c.beginText(72, 690)
    for line in (
        "The roots of a quadratic polynomial are determined entirely by its",
        "coefficients. For real coefficients the discriminant decides whether the",
        "roots are real or complex conjugates. The closed form is given by the",
        "well known quadratic formula shown below as a display equation.",
    ):
        text.textLine(line)
    c.drawText(text)

    # Display equation: x = (-b +/- sqrt(b^2 - 4ac)) / 2a with a drawn
    # fraction bar and radical so it is visually unambiguous mathematics.
    eq_cx = width / 2
    eq_y = 590
    c.setFont("Helvetica", 14)
    c.drawString(eq_cx - 110, eq_y, "x =")
    # Numerator with a superscript 2 on b.
    c.setFont("Helvetica", 13)
    c.drawString(eq_cx - 62, eq_y + 12, "−b ± √b² − 4ac")
    # Radical overbar.
    c.line(eq_cx - 24, eq_y + 25, eq_cx + 42, eq_y + 25)
    # Fraction bar.
    c.line(eq_cx - 70, eq_y + 6, eq_cx + 50, eq_y + 6)
    # Denominator.
    c.drawCentredString(eq_cx - 10, eq_y - 12, "2a")

    c.setFont("Helvetica", 11)
    text = c.beginText(72, 540)
    for line in (
        "When the discriminant is negative the square root is imaginary and the",
        "two roots form a complex conjugate pair. This concludes the example.",
    ):
        text.textLine(line)
    c.drawText(text)
    c.save()
    print(f"Wrote {out} ({out.stat().st_size} bytes)")


def make_tables() -> None:
    """A table-heavy one-page report: prose plus two ruled data tables."""
    letter, _canvas = _reportlab()
    from reportlab.lib import colors
    from reportlab.lib.styles import getSampleStyleSheet
    from reportlab.platypus import Paragraph, SimpleDocTemplate, Spacer, Table, TableStyle

    out = HERE / "tables.pdf"
    styles = getSampleStyleSheet()
    grid = TableStyle(
        [
            ("GRID", (0, 0), (-1, -1), 0.7, colors.black),
            ("BACKGROUND", (0, 0), (-1, 0), colors.whitesmoke),
            ("FONTNAME", (0, 0), (-1, 0), "Helvetica-Bold"),
            ("FONTNAME", (0, 1), (-1, -1), "Helvetica"),
            ("FONTSIZE", (0, 0), (-1, -1), 10),
            ("ALIGN", (1, 1), (-1, -1), "RIGHT"),
        ]
    )
    story = [
        Paragraph("Quarterly Generation Report", styles["Title"]),
        Paragraph(
            "Output by region and technology for the reporting year. All figures "
            "are in gigawatt hours and are provisional until audited.",
            styles["BodyText"],
        ),
        Spacer(1, 12),
        Table(
            [
                ["Region", "Q1", "Q2", "Q3", "Q4"],
                ["North", "1214", "1180", "1105", "1298"],
                ["South", "986", "1002", "1044", "975"],
                ["East", "1530", "1488", "1512", "1575"],
                ["West", "874", "861", "902", "890"],
            ],
            style=grid,
        ),
        Spacer(1, 18),
        Paragraph(
            "The second table breaks the same output down by technology.",
            styles["BodyText"],
        ),
        Spacer(1, 12),
        Table(
            [
                ["Technology", "Capacity MW", "Output GWh", "Load factor"],
                ["Wind", "2400", "6318", "0.30"],
                ["Solar", "1800", "1735", "0.11"],
                ["Biomass", "640", "4483", "0.80"],
            ],
            style=grid,
        ),
    ]
    SimpleDocTemplate(str(out), pagesize=letter).build(story)
    print(f"Wrote {out} ({out.stat().st_size} bytes)")


def make_twocolumn() -> None:
    """A two-column paper-style page.

    Each column carries a distinctive full sentence; a layout-aware
    extractor keeps each sentence contiguous, while naive left-to-right
    extraction interleaves lines across the column gap.
    """
    letter, _canvas = _reportlab()
    from reportlab.lib.styles import getSampleStyleSheet
    from reportlab.platypus import BaseDocTemplate, Frame, PageTemplate, Paragraph, Spacer

    out = HERE / "twocolumn.pdf"
    width, height = letter
    margin, gap = 72, 24
    col_w = (width - 2 * margin - gap) / 2
    frames = [
        Frame(margin, margin, col_w, height - 2 * margin, id="left"),
        Frame(margin + col_w + gap, margin, col_w, height - 2 * margin, id="right"),
    ]
    doc = BaseDocTemplate(str(out), pagesize=letter)
    doc.addPageTemplates([PageTemplate(id="twocol", frames=frames)])

    styles = getSampleStyleSheet()
    body = styles["BodyText"]
    left_sentence = (
        "The aardvark population of the western valley increased steadily "
        "throughout the survey period, defying every published model."
    )
    right_sentence = (
        "Meanwhile the barnacle colonies of the eastern shoreline declined "
        "sharply, a collapse the tidal records had clearly foreshadowed."
    )
    filler = (
        "Observations were collected daily at dawn by the resident field team "
        "using the standard protocol adopted in the second season. "
    )
    story = [
        Paragraph("A Study in Columns", styles["Heading1"]),
        Paragraph(filler * 6, body),
        Spacer(1, 8),
        Paragraph(left_sentence, body),
        Spacer(1, 8),
        Paragraph(filler * 8, body),
        Spacer(1, 8),
        Paragraph(right_sentence, body),
        Spacer(1, 8),
        Paragraph(filler * 6, body),
    ]
    doc.build(story)
    print(f"Wrote {out} ({out.stat().st_size} bytes)")


def make_multipage() -> None:
    """A genuine three-page prose document.

    Each page carries a distinctive sentence plus enough filler to clear
    the low-text floor, so `--- p.N ---` anchor assembly and page-boundary
    truncation can be asserted against real page breaks.
    """
    letter, _canvas = _reportlab()
    from reportlab.lib.styles import getSampleStyleSheet
    from reportlab.platypus import PageBreak, Paragraph, SimpleDocTemplate, Spacer

    out = HERE / "multipage.pdf"
    styles = getSampleStyleSheet()
    body = styles["BodyText"]
    filler = (
        "This paragraph exists purely to give the page a healthy body of "
        "prose so that no page trips the low-text completeness floor. "
    )
    pages = [
        ("A Document of Three Pages", "The albatross glided over page one without ever landing."),
        ("The Middle Passage", "A bewildered badger burrowed straight through page two."),
        ("Concluding Remarks", "The capybara concluded matters calmly on page three."),
    ]
    story = []
    for i, (heading, sentence) in enumerate(pages):
        if i > 0:
            story.append(PageBreak())
        story.append(Paragraph(heading, styles["Heading1"]))
        story.append(Paragraph(filler * 4, body))
        story.append(Spacer(1, 8))
        story.append(Paragraph(sentence, body))
        story.append(Spacer(1, 8))
        story.append(Paragraph(filler * 4, body))
    SimpleDocTemplate(str(out), pagesize=letter).build(story)
    print(f"Wrote {out} ({out.stat().st_size} bytes)")


if __name__ == "__main__":
    make_hello()
    make_scanned()
    make_equation()
    make_tables()
    make_twocolumn()
    make_multipage()
