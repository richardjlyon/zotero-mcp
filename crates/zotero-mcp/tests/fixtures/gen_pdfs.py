#!/usr/bin/env python3
"""Generate test PDFs for the pdf-extraction-fallback tests.

Requires:  pip install pikepdf

Run from anywhere; produces files next to this script.
"""
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


if __name__ == "__main__":
    make_hello()
