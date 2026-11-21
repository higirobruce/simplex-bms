// Header and example rows mirror ProductImportRowSchema (validations.ts). Blank optional cells
// fall back to defaults: taxTyCd B, isWeighed false, department General. The example values are
// illustrative only; set each product's real EBM tax code (A exempt, B 18% VAT, C, D).
export const PRODUCT_IMPORT_TEMPLATE =
  "sku,name,category,unitPrice,costPrice,reorderLevel,barcode,plu,taxTyCd,department,isWeighed,packSize,shelfLifeDays\n" +
  "MLK-001,Inyange Milk 1L,Dairy,1200,950,24,6001234567890,,B,Dairy,false,1L,7\n" +
  "TOM-001,Tomatoes,Produce,1500,1100,10,,4012,A,Fresh produce,true,,5\n";
