## MODIFIED Requirements

### Requirement: Labelled route and preserved fallback

Extraction SHALL try the layout-aware primary route first and SHALL always identify which
engine produced the result. Degradation SHALL be observable in the result, never silent.

Where a layout route is **configured but unavailable**, flat-text output SHALL NOT be
delivered as an ordinary success. Such a call SHALL fail loudly, naming the unavailable
layout route and the opt-in as the remedy, unless the caller has explicitly opted in to
degraded output for that call. Where the caller has opted in, the flat-text chain runs as
before and the result is labelled and marked incomplete as today. Correctness SHALL NOT
depend on a caller inspecting an optional field on a successful response.

Where **no layout route is configured**, behaviour is unchanged: the local flat-text chain
runs, is labelled, and is marked incomplete. This state SHALL be distinguishable from the
configured-but-unavailable state, so that hosts with no layout service — including
continuous integration — remain fully functional.

The degradation marker SHALL be carried in the result envelope and in the error, and SHALL
NOT be injected into the extracted text. The extracted text is the artefact downstream
fact-checking treats as authoritative; it SHALL contain only what the document contains.

An explicit request for plain output (`plain`) is a caller choosing the flat-text route
deliberately and SHALL continue to succeed unchanged.

#### Scenario: Layout route configured but cold, no opt-in

- **WHEN** a caller who has not opted in to degraded output extracts an item on a host
  where the layout service is configured but fails its health check
- **THEN** the call returns a loud error naming the unavailable layout route and the
  opt-in as the remedy — it does not return flat text as a success

#### Scenario: Layout route configured but cold, caller opted in

- **WHEN** the same call is made with the degraded-output opt-in set
- **THEN** extraction proceeds via the local flat-text chain, the result's engine label
  reflects the fallback, and the completeness report marks it incomplete

#### Scenario: No layout route configured

- **WHEN** extraction runs on a host with no layout service configured at all
- **THEN** the local flat-text chain runs and succeeds without an opt-in, labelled and
  marked incomplete, exactly as before this change

#### Scenario: Plain output preserved

- **WHEN** a caller explicitly requests `plain` extraction
- **THEN** the previous flat-text behaviour is used, `format` is plain, and no
  degraded-output opt-in is required

#### Scenario: The text is not annotated

- **WHEN** any degraded extraction returns text to a caller
- **THEN** the returned text contains no marker, banner or warning added by the server —
  every sentence in it came from the document, and the degradation is reported alongside

#### Scenario: Same item, both routes, one contract

- **WHEN** an item is extracted while the layout service is cold and again once it is warm,
  without the opt-in
- **THEN** the first call fails loudly and the second succeeds with markdown — the two
  outcomes are never both successes distinguished only by a metadata field

## ADDED Requirements

### Requirement: Table-fidelity regression over committed fixtures

The change SHALL ship a regression test over committed fixtures asserting what actually
distinguishes the routes: tables present versus tables absent. Assertions SHALL be on table
structure and count, never on character count, because the observed loss is structural
while volume is comparable (page 1 of the evidenced item: 3,496 characters flat versus
3,299 markdown). Tests requiring the layout service SHALL skip loudly when it is absent.

#### Scenario: Comparable volume, different fidelity

- **WHEN** the same table-bearing fixture is extracted on the layout route and on the
  flat-text route
- **THEN** the layout result contains the document's tables as markdown tables with rows
  and columns intact, the flat result does not, and a test distinguishing them by
  character count alone would not detect the difference

#### Scenario: Table count is asserted, not inferred

- **WHEN** the table-heavy fixture is extracted on the layout route
- **THEN** the number of tables in the output is asserted directly against the fixture's
  known table count

#### Scenario: All three postures are covered

- **WHEN** the regression suite runs
- **THEN** it covers a host with no layout route configured, a configured-but-unavailable
  route without the opt-in, and the same with the opt-in — and no case passes by
  extracting nothing
