# PDF layout fixtures

- `pdf-positioned-cells.pdf`: synthetic Helvetica table using positioned glyph runs inside TJ text objects. Covers independent translation of headings and descriptions while numeric cells retain their coordinates.
- `pdf-positioned-cid-cells.pdf`: synthetic two-page counterpart with an embedded subset of DejaVu Sans, including an unchanged numeric row. Covers reuse of the original subset font when another cell in the same text object changes.
- `DEJAVU-LICENSE`: license for the subset font in the CID fixture.

These files contain synthetic examples only. Private documents used for local comparison are not included.
