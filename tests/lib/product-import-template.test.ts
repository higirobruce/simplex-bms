import { expect, test } from "vitest";
import { parseCsv } from "@/lib/csv";
import { PRODUCT_IMPORT_TEMPLATE } from "@/lib/product-import-template";
import { ProductImportRowSchema } from "@/lib/validations";

test("the import template names every grocery column the importer accepts", () => {
  const header = PRODUCT_IMPORT_TEMPLATE.split("\n")[0].split(",");
  for (const col of ["barcode", "plu", "taxTyCd", "department", "isWeighed", "packSize", "shelfLifeDays"]) {
    expect(header).toContain(col);
  }
  // and nothing the importer would ignore: every header is a key of the row schema
  expect(Object.keys(ProductImportRowSchema.shape)).toEqual(expect.arrayContaining(header));
});

test("the template's example rows import as written", () => {
  const rows = parseCsv(PRODUCT_IMPORT_TEMPLATE);
  expect(rows).toHaveLength(2);
  const [milk, tomatoes] = rows.map((r) => ProductImportRowSchema.parse(r));
  expect(milk).toMatchObject({ taxTyCd: "B", isWeighed: false, barcode: "6001234567890", shelfLifeDays: 7 });
  expect(tomatoes).toMatchObject({ taxTyCd: "A", isWeighed: true, plu: "4012", department: "Fresh produce" });
});
