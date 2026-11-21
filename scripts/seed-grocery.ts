/**
 * Seeds a Rwandan supermarket: a full department spread of grocery lines with shelf
 * stock, batches on the perishables, two promotions, three lanes and a trading day of
 * real baskets — weighed produce, VAT-exempt staples and split tenders included.
 *
 * Conventions this seed encodes (all from the Phase 1 global constraints):
 *   - Money is Decimal(12, 2) in RWF; shelf prices are VAT-inclusive, as on a Rwandan
 *     shelf edge, so the EBM totals carve the 18% out of the price rather than add it.
 *   - Quantity is Decimal(12, 3). Weighed goods are priced per kg and sold in kg.
 *   - taxTyCd "A" is A-EX (exempt), "B" is the 18% standard rate. See EXEMPTION_RULE.
 *   - Tenders use TenderMethod. Mobile money is *recorded, never confirmed*, in v1:
 *     providerRef and confirmed are left at their defaults.
 *   - Discounts are a discountKind plus a discountValue, at line and basket level only.
 *
 * Run directly with `npm run db:seed:grocery`.
 */
import { basename } from "node:path";
import { Prisma, PrismaClient } from "@prisma/client";
import bcrypt from "bcryptjs";
import { lineAmount, toQty } from "../src/lib/qty";

/**
 * EXEMPTION_RULE — how taxTyCd is assigned, so it can be retuned in one place.
 *
 * "A" (A-EX) is given to the goods Rwandan VAT treats as exempt staples: unprocessed
 * agricultural, livestock and fishery produce as it leaves the farm or the butcher's
 * block (fresh fruit and vegetables, raw meat and fish, eggs), milk (fresh and UHT,
 * plain), the milled staple flours, grains and pulses a household cooks from, and
 * menstrual products, which Rwanda removed VAT from in 2019.
 *
 * "B" (18%) is everything else: anything baked, bottled, cooked, cured, sweetened or
 * otherwise processed (bread, yoghurt, cheese, butter, sausages, juice, sugar, oil,
 * pasta), and every non-food line. The exemption follows the goods, not the aisle —
 * raw beef is exempt while the sausages beside it are not.
 *
 * The precise statutory list is a tax-configuration decision for the shop; this is a
 * defensible default that exercises both codes across the catalogue.
 */

// ===========================================================================
// The catalogue. Data only — the rows are built from it further down.
// `plu` present means weighed (priced per kg, no barcode); otherwise the line is
// packaged and gets an EAN-13. `exempt` means taxTyCd "A". `from` picks the GS1
// prefix, defaulting to Rwanda.
// ===========================================================================

type Origin = "RW" | "KE" | "TZ" | "UG" | "ZA" | "IMP";

type Item = {
  name: string;
  /** VAT-inclusive shelf price in RWF; per kg for weighed lines. */
  price: number;
  /** 4-digit till code. Present => weighed. */
  plu?: string;
  pack?: string;
  /** Shelf life in days (Task 9 bounds it to 1..3650). */
  life?: number;
  exempt?: true;
  from?: Origin;
};

type Department = {
  name: string;
  /** SKU prefix — deliberately unlike the legacy B2B prefixes (HW-, CHR-, DSK-...). */
  sku: string;
  /** Gross margin used to derive costPrice. */
  margin: number;
  reorderLevel: number;
  items: Item[];
};

const CATALOGUE: Department[] = [
  {
    name: "Fresh Produce",
    sku: "FP",
    margin: 0.3,
    reorderLevel: 15,
    items: [
      { name: "Tomatoes", price: 1200, plu: "4087", life: 7, exempt: true },
      { name: "Onions, red", price: 1500, plu: "4663", life: 21, exempt: true },
      { name: "Irish potatoes, Kinigi", price: 700, plu: "4072", life: 30, exempt: true },
      { name: "Sweet potatoes", price: 600, plu: "4091", life: 21, exempt: true },
      { name: "Cassava root", price: 500, plu: "4584", life: 14, exempt: true },
      { name: "Carrots", price: 1000, plu: "4562", life: 21, exempt: true },
      { name: "Cabbage, green", price: 600, plu: "4069", life: 21, exempt: true },
      { name: "Green pepper", price: 2500, plu: "4065", life: 10, exempt: true },
      { name: "Hot chilli pepper", price: 3000, plu: "4688", life: 10, exempt: true },
      { name: "Aubergine", price: 1200, plu: "4081", life: 10, exempt: true },
      { name: "Avocado, Hass", price: 1500, plu: "4225", life: 7, exempt: true },
      { name: "Sweet bananas", price: 1000, plu: "4011", life: 7, exempt: true },
      { name: "Cooking bananas, matoke", price: 900, plu: "4235", life: 10, exempt: true },
      { name: "Pineapple", price: 1400, plu: "4430", life: 7, exempt: true },
      { name: "Mango, Ngowe", price: 2000, plu: "4051", life: 7, exempt: true },
      { name: "Passion fruit", price: 3500, plu: "4397", life: 10, exempt: true },
      { name: "Tree tomato, ibinyomoro", price: 2500, plu: "4799", life: 7, exempt: true },
      { name: "Oranges", price: 1800, plu: "4013", life: 14, exempt: true },
      { name: "Lemons", price: 2500, plu: "4053", life: 14, exempt: true },
      { name: "Watermelon", price: 800, plu: "4032", life: 10, exempt: true },
      { name: "Apples, Royal Gala", price: 4500, plu: "4133", life: 21, exempt: true },
      { name: "Garlic", price: 4000, plu: "4608", life: 60, exempt: true },
      { name: "Ginger", price: 3500, plu: "4612", life: 30, exempt: true },
      { name: "French beans", price: 2000, plu: "4534", life: 7, exempt: true },
      { name: "Green peas, shelled", price: 2500, plu: "4725", life: 5, exempt: true },
      { name: "Button mushrooms", price: 2500, pack: "250 g punnet", life: 7, exempt: true },
      { name: "Mixed salad leaves", price: 1800, pack: "200 g pack", life: 5, exempt: true },
      { name: "Cherry tomatoes", price: 1500, pack: "250 g punnet", life: 7, exempt: true },
      { name: "Garden herbs, rosemary & thyme", price: 1200, pack: "50 g pack", life: 7, exempt: true },
    ],
  },
  {
    name: "Butchery",
    sku: "BU",
    margin: 0.22,
    reorderLevel: 10,
    items: [
      { name: "Beef rump steak", price: 7500, plu: "2010", life: 4, exempt: true },
      { name: "Beef stewing cubes", price: 6000, plu: "2011", life: 4, exempt: true },
      { name: "Beef mince", price: 6500, plu: "2012", life: 3, exempt: true },
      { name: "Beef short ribs", price: 5800, plu: "2013", life: 4, exempt: true },
      { name: "Goat meat, bone-in", price: 7000, plu: "2020", life: 4, exempt: true },
      { name: "Pork chops", price: 6500, plu: "2030", life: 4, exempt: true },
      { name: "Pork belly", price: 6000, plu: "2031", life: 4, exempt: true },
      { name: "Whole chicken, fresh", price: 5500, plu: "2040", life: 3, exempt: true },
      { name: "Chicken breast fillet", price: 8000, plu: "2041", life: 3, exempt: true },
      { name: "Chicken drumsticks", price: 5000, plu: "2042", life: 3, exempt: true },
      { name: "Tilapia, whole fresh", price: 6500, plu: "2050", life: 2, exempt: true },
      { name: "Sambaza, lake sardines", price: 4000, plu: "2051", life: 2, exempt: true },
      { name: "Nile perch fillet", price: 9000, plu: "2052", life: 2, exempt: true },
      { name: "Beef sausages", price: 4500, pack: "500 g", life: 10 },
      { name: "Smoked beef bacon", price: 5500, pack: "250 g", life: 21 },
      { name: "Chicken franks", price: 3800, pack: "400 g", life: 14 },
    ],
  },
  {
    name: "Dairy",
    sku: "DA",
    margin: 0.18,
    reorderLevel: 24,
    items: [
      { name: "Inyange fresh milk", price: 1200, pack: "1 L", life: 7, exempt: true },
      { name: "Inyange UHT milk", price: 1500, pack: "1 L", life: 180, exempt: true },
      { name: "Mukamira fresh milk", price: 700, pack: "500 ml", life: 7, exempt: true },
      { name: "Inyange milk sachet", price: 650, pack: "500 ml", life: 5, exempt: true },
      { name: "Brookside UHT milk", price: 1600, pack: "1 L", life: 180, exempt: true, from: "KE" },
      { name: "Eggs, tray of 30", price: 7500, pack: "30 eggs", life: 21, exempt: true },
      { name: "Eggs, pack of 6", price: 1700, pack: "6 eggs", life: 21, exempt: true },
      { name: "Inyange natural yoghurt", price: 1800, pack: "500 ml", life: 14 },
      { name: "Inyange drinking yoghurt, strawberry", price: 800, pack: "250 ml", life: 14 },
      { name: "Nyirangarama drinking yoghurt", price: 1500, pack: "500 ml", life: 14 },
      { name: "Gouda cheese", price: 5500, pack: "250 g", life: 60 },
      { name: "Mozzarella", price: 6000, pack: "200 g", life: 30 },
      { name: "Processed cheese slices", price: 4500, pack: "200 g", life: 60 },
      { name: "Butter, unsalted", price: 4500, pack: "250 g", life: 60 },
      { name: "Blue Band margarine", price: 3500, pack: "500 g", life: 180, from: "KE" },
      { name: "Fresh cream", price: 2500, pack: "250 ml", life: 14 },
      { name: "Nido milk powder", price: 8500, pack: "400 g", life: 365, from: "IMP" },
      { name: "Condensed milk", price: 2800, pack: "397 g", life: 365 },
    ],
  },
  {
    name: "Bakery",
    sku: "BK",
    margin: 0.35,
    reorderLevel: 20,
    items: [
      { name: "Sliced white bread", price: 1800, pack: "700 g", life: 4 },
      { name: "Sliced brown bread", price: 1900, pack: "700 g", life: 4 },
      { name: "Whole wheat bread", price: 2200, pack: "600 g", life: 4 },
      { name: "Baguette", price: 1000, pack: "1 loaf", life: 2 },
      { name: "Bread rolls", price: 1500, pack: "6 pack", life: 3 },
      { name: "Mandazi", price: 1200, pack: "6 pack", life: 2 },
      { name: "Chapati", price: 1500, pack: "5 pack", life: 2 },
      { name: "Butter croissant", price: 1800, pack: "2 pack", life: 2 },
      { name: "Doughnuts", price: 1600, pack: "4 pack", life: 2 },
      { name: "Vanilla sponge cake", price: 15000, pack: "1 kg", life: 5 },
      { name: "Banana bread loaf", price: 2500, pack: "400 g", life: 4 },
      { name: "Shortbread biscuits", price: 2800, pack: "300 g", life: 180 },
    ],
  },
  {
    name: "Dry Goods",
    sku: "DG",
    margin: 0.15,
    reorderLevel: 30,
    items: [
      { name: "Minimex maize flour", price: 2600, pack: "2 kg", life: 365, exempt: true },
      { name: "Maize flour, kawunga", price: 1400, pack: "1 kg", life: 365, exempt: true },
      { name: "Cassava flour", price: 1800, pack: "1 kg", life: 365, exempt: true },
      { name: "Sorghum flour", price: 2000, pack: "1 kg", life: 365, exempt: true },
      { name: "Millet flour", price: 2400, pack: "1 kg", life: 365, exempt: true },
      { name: "Azam wheat flour", price: 2800, pack: "2 kg", life: 270, exempt: true, from: "TZ" },
      { name: "Rice, Pakistani long grain", price: 9500, pack: "5 kg", life: 540, exempt: true },
      { name: "Rice, Tanzanian aromatic", price: 1900, pack: "1 kg", life: 540, exempt: true, from: "TZ" },
      { name: "Dry beans, red kidney", price: 1500, pack: "1 kg", life: 365, exempt: true },
      { name: "Dry beans, yellow", price: 1600, pack: "1 kg", life: 365, exempt: true },
      { name: "Yellow split peas", price: 1800, pack: "1 kg", life: 365, exempt: true },
      { name: "Groundnuts, raw", price: 2500, pack: "500 g", life: 180, exempt: true },
      { name: "Soya flour", price: 2200, pack: "1 kg", life: 270, exempt: true },
      { name: "Kabuye white sugar", price: 1800, pack: "1 kg", life: 720 },
      { name: "Brown sugar", price: 2200, pack: "1 kg", life: 720 },
      { name: "Table salt, iodised", price: 500, pack: "1 kg", life: 1095 },
      { name: "Sunflower cooking oil", price: 2800, pack: "1 L", life: 365 },
      { name: "Palm cooking oil", price: 11000, pack: "3 L", life: 365 },
      { name: "Spaghetti", price: 1200, pack: "500 g", life: 540 },
      { name: "Macaroni", price: 1300, pack: "500 g", life: 540 },
      { name: "Royco beef cubes", price: 1000, pack: "10 x 8 g", life: 540, from: "KE" },
      { name: "Tomato paste", price: 1500, pack: "400 g", life: 540 },
      { name: "Peanut butter", price: 3500, pack: "400 g", life: 270 },
      { name: "Weetabix", price: 6500, pack: "450 g", life: 365, from: "KE" },
      { name: "Instant porridge mix", price: 2500, pack: "500 g", life: 270 },
      { name: "Curry powder", price: 1200, pack: "100 g", life: 540 },
      { name: "Baking powder", price: 800, pack: "100 g", life: 540 },
    ],
  },
  {
    name: "Beverages",
    sku: "BV",
    margin: 0.2,
    reorderLevel: 36,
    items: [
      { name: "Inyange mango juice", price: 1800, pack: "1 L", life: 270 },
      { name: "Inyange orange juice", price: 1800, pack: "1 L", life: 270 },
      { name: "Inyange passion juice", price: 1000, pack: "500 ml", life: 270 },
      { name: "Inyange still water", price: 700, pack: "1.5 L", life: 720 },
      { name: "Inyange still water", price: 400, pack: "500 ml", life: 720 },
      { name: "Inyange sparkling water", price: 900, pack: "500 ml", life: 720 },
      { name: "Coca-Cola", price: 700, pack: "500 ml PET", life: 270 },
      { name: "Fanta Orange", price: 700, pack: "500 ml PET", life: 270 },
      { name: "Sprite", price: 1200, pack: "1 L PET", life: 270 },
      { name: "Coca-Cola, returnable", price: 500, pack: "300 ml glass", life: 270 },
      { name: "Primus", price: 1200, pack: "500 ml", life: 365 },
      { name: "Mutzig", price: 1500, pack: "500 ml", life: 365 },
      { name: "Skol Lager", price: 1300, pack: "500 ml", life: 365 },
      { name: "Turbo King", price: 1400, pack: "500 ml", life: 365 },
      { name: "Rwanda Mountain Tea leaves", price: 1800, pack: "250 g", life: 720 },
      { name: "Africafe instant coffee", price: 3500, pack: "100 g", life: 720, from: "TZ" },
      { name: "Rwacof ground coffee", price: 4500, pack: "250 g", life: 365 },
      { name: "Red Bull", price: 1800, pack: "250 ml", life: 365, from: "IMP" },
      { name: "Four Cousins red wine", price: 12000, pack: "750 ml", life: 1095, from: "ZA" },
      { name: "Waragi gin", price: 8000, pack: "750 ml", life: 1095, from: "UG" },
    ],
  },
  {
    name: "Household",
    sku: "HH",
    margin: 0.25,
    reorderLevel: 18,
    items: [
      { name: "Omo washing powder", price: 3500, pack: "1 kg" },
      { name: "Omo washing powder", price: 2000, pack: "500 g" },
      { name: "Sunlight laundry bar", price: 1500, pack: "800 g" },
      { name: "Dishwashing liquid", price: 2200, pack: "750 ml" },
      { name: "Vim scouring powder", price: 2200, pack: "500 g" },
      { name: "Jik bleach", price: 1800, pack: "750 ml" },
      { name: "Harpic toilet cleaner", price: 3500, pack: "500 ml", from: "ZA" },
      { name: "Toilet paper", price: 4500, pack: "10 rolls" },
      { name: "Kitchen towel", price: 2500, pack: "2 rolls" },
      { name: "Doom insect spray", price: 4500, pack: "400 ml", from: "ZA" },
      { name: "Mosquito coils", price: 1000, pack: "10 pack" },
      { name: "Candles", price: 1200, pack: "6 pack" },
      { name: "Safety matches", price: 500, pack: "10 boxes" },
      { name: "Bin liners", price: 2500, pack: "30 pack" },
      { name: "Scouring pads", price: 1000, pack: "3 pack" },
      { name: "Aluminium foil", price: 3500, pack: "10 m" },
    ],
  },
  {
    name: "Personal Care",
    sku: "PC",
    margin: 0.28,
    reorderLevel: 18,
    items: [
      { name: "Colgate toothpaste", price: 2500, pack: "140 ml", life: 1095 },
      { name: "Toothbrush, medium", price: 1200, pack: "1 brush" },
      { name: "Nivea body lotion", price: 7500, pack: "400 ml", life: 1095, from: "IMP" },
      { name: "Vaseline petroleum jelly", price: 3500, pack: "250 ml", life: 1095, from: "ZA" },
      { name: "Protex bar soap", price: 1200, pack: "150 g" },
      { name: "Lux bar soap", price: 1000, pack: "175 g" },
      { name: "Geisha bar soap", price: 900, pack: "125 g", from: "KE" },
      { name: "Dove beauty bar", price: 2500, pack: "100 g", from: "IMP" },
      { name: "Sunsilk shampoo", price: 4500, pack: "400 ml", life: 1095, from: "KE" },
      { name: "Hair conditioner", price: 3800, pack: "250 ml", life: 1095 },
      { name: "Always sanitary pads", price: 1500, pack: "10 pack", exempt: true },
      { name: "Softcare sanitary pads", price: 1200, pack: "8 pack", exempt: true, from: "KE" },
      { name: "Pampers baby diapers", price: 12000, pack: "30 pack", from: "IMP" },
      { name: "Baby wipes", price: 2500, pack: "72 pack", life: 1095 },
      { name: "Nivea roll-on deodorant", price: 3500, pack: "50 ml", life: 1095, from: "IMP" },
      { name: "Disposable razors", price: 1500, pack: "5 pack" },
      { name: "Hand sanitiser", price: 3000, pack: "500 ml", life: 1095 },
      { name: "Facial tissues", price: 1500, pack: "100 sheets" },
    ],
  },
];

// ===========================================================================
// Helpers
// ===========================================================================

/** GS1 member prefixes: Rwanda is 632; imports carry their own country's prefix. */
const GS1_PREFIX: Record<Origin, string> = {
  RW: "632",
  KE: "616",
  TZ: "620",
  UG: "605",
  ZA: "600",
  IMP: "400",
};

/** A distinct company block per origin, so the barcodes do not all look alike. */
const GS1_COMPANY: Record<Origin, string> = {
  RW: "4580",
  KE: "1023",
  TZ: "5017",
  UG: "2204",
  ZA: "7311",
  IMP: "6642",
};

/** GS1 mod-10 check digit: weights 1 and 3 from the left over the first 12 digits. */
function ean13(origin: Origin, serial: number): string {
  const body = GS1_PREFIX[origin] + GS1_COMPANY[origin] + String(serial).padStart(5, "0");
  let sum = 0;
  for (let i = 0; i < 12; i++) sum += Number(body[i]) * (i % 2 === 0 ? 1 : 3);
  return body + String((10 - (sum % 10)) % 10);
}

/** Seeded PRNG, so a reseed produces the same shop twice. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const money = (v: Prisma.Decimal | string | number) => new Prisma.Decimal(v).toDecimalPlaces(2);
/** The 18% VAT carved out of a tax-inclusive amount. */
const vatOf = (inclusive: Prisma.Decimal) => inclusive.times(18).div(118).toDecimalPlaces(2);

const DAY_MS = 86_400_000;
const atHour = (base: Date, hour: number, minute = 0) =>
  new Date(base.getFullYear(), base.getMonth(), base.getDate(), hour, minute, 0, 0);

// ===========================================================================
// seedGrocery
// ===========================================================================

/** The slice of the client this seed writes through (so tests can pass their own). */
type SeedClient = Pick<
  PrismaClient,
  | "user"
  | "location"
  | "product"
  | "stockLevel"
  | "stockMovement"
  | "batch"
  | "stockWriteOff"
  | "promotion"
  | "priceChange"
  | "lane"
  | "shift"
  | "sale"
>;

let lazyClient: PrismaClient | null = null;
function defaultDb(): PrismaClient {
  if (!lazyClient) lazyClient = new PrismaClient();
  return lazyClient;
}

type SeededProduct = {
  id: string;
  name: string;
  department: string;
  unitPrice: Prisma.Decimal;
  isWeighed: boolean;
  taxTyCd: "A" | "B";
};

/**
 * Stocks an existing org as a supermarket. Creates rows only — never deletes — and is
 * not idempotent: a second call collides on the SKU and barcode unique constraints,
 * which is what the direct-run guard below checks for.
 *
 * The org must already have a user (the cashier who opens the shifts). A location is
 * reused if the org has one, otherwise a shop floor is created.
 */
export async function seedGrocery(orgId: string, db: SeedClient = defaultDb()): Promise<void> {
  const cashier = await db.user.findFirst({ where: { orgId }, orderBy: { createdAt: "asc" } });
  if (!cashier) {
    throw new Error(`seedGrocery: org ${orgId} has no user to open a shift with; create one first.`);
  }
  const location =
    (await db.location.findFirst({ where: { orgId }, orderBy: { createdAt: "asc" } })) ??
    (await db.location.create({ data: { name: "Shop Floor", code: "SHOP", orgId } }));

  const rng = mulberry32(20261121);

  // ---- products -----------------------------------------------------------
  let serial = 10_001;
  const rows: Prisma.ProductCreateManyInput[] = [];
  for (const dept of CATALOGUE) {
    dept.items.forEach((item, i) => {
      const weighed = item.plu !== undefined;
      rows.push({
        sku: `${dept.sku}-${String(i + 1).padStart(3, "0")}`,
        name: item.name,
        // category is the legacy B2B field the existing screens group by; keeping it in
        // step with department means those screens read correctly too.
        category: dept.name,
        department: dept.name,
        unitPrice: money(item.price).toFixed(2),
        costPrice: money(item.price * (1 - dept.margin)).toFixed(2),
        reorderLevel: toQty(dept.reorderLevel).toFixed(3),
        barcode: weighed ? null : ean13(item.from ?? "RW", serial++),
        plu: item.plu ?? null,
        isWeighed: weighed,
        taxTyCd: item.exempt ? "A" : "B",
        packSize: item.pack ?? null,
        shelfLifeDays: item.life ?? null,
        orgId,
      });
    });
  }
  await db.product.createMany({ data: rows });

  const products: SeededProduct[] = (
    await db.product.findMany({
      where: { orgId, department: { not: "General" } },
      select: {
        id: true,
        name: true,
        department: true,
        unitPrice: true,
        isWeighed: true,
        taxTyCd: true,
      },
      orderBy: { sku: "asc" },
    })
  ).map((p) => ({ ...p, taxTyCd: p.taxTyCd as "A" | "B" }));

  // ---- shelf stock --------------------------------------------------------
  // A snapshot of what is on the floor today. Phase 2 owns movements out of stock, so
  // the sales below deliberately do not draw these down.
  await db.stockLevel.createMany({
    data: products.map((p) => ({
      productId: p.id,
      locationId: location.id,
      qty: p.isWeighed
        ? toQty(8 + rng() * 40).toDecimalPlaces(3).toFixed(3)
        : toQty(Math.floor(12 + rng() * 140)).toFixed(3),
    })),
  });

  // ---- batches, write-offs and a couple of receipts -----------------------
  const now = new Date();
  /** The catalogue line by name, so these fixtures survive a reordered catalogue. */
  const item = (name: string): SeededProduct => {
    const found = products.find((p) => p.name === name);
    if (!found) throw new Error(`seedGrocery: no catalogue line named "${name}"`);
    return found;
  };
  const perishables = [
    { product: item("Inyange fresh milk"), lot: "MLK-2611-A", bestBeforeDays: 5, qty: "48.000" },
    { product: item("Inyange natural yoghurt"), lot: "YOG-2611-B", bestBeforeDays: 11, qty: "36.000" },
    { product: item("Sliced white bread"), lot: "BRD-2611-C", bestBeforeDays: 3, qty: "40.000" },
    { product: item("Beef mince"), lot: "MNC-2611-D", bestBeforeDays: 2, qty: "18.500" },
    { product: item("Gouda cheese"), lot: "CHS-2610-E", bestBeforeDays: 45, qty: "12.000" },
    // Already past its date: the write-off below clears it.
    { product: item("Mukamira fresh milk"), lot: "MLK-2609-F", bestBeforeDays: -3, qty: "24.000" },
  ];
  const batches = [];
  for (const p of perishables) {
    batches.push(
      await db.batch.create({
        data: {
          productId: p.product.id,
          lot: p.lot,
          receivedAt: new Date(now.getTime() - 4 * DAY_MS),
          bestBefore: new Date(now.getTime() + p.bestBeforeDays * DAY_MS),
          orgId,
        },
      }),
    );
  }
  const expired = batches[batches.length - 1];
  await db.stockWriteOff.createMany({
    data: [
      {
        productId: perishables[perishables.length - 1].product.id,
        locationId: location.id,
        batchId: expired.id,
        qty: "6.000",
        reason: "EXPIRY",
        note: "Past best-before on the morning check",
        orgId,
      },
      {
        productId: item("Jik bleach").id,
        locationId: location.id,
        qty: "2.000",
        reason: "DAMAGE",
        note: "Bottles split in the back store",
        orgId,
      },
    ],
  });
  await db.stockMovement.createMany({
    data: [
      {
        productId: perishables[0].product.id,
        toLocationId: location.id,
        qty: perishables[0].qty,
        type: "RECEIPT",
        reason: "Morning dairy delivery",
        batchId: batches[0].id,
        orgId,
      },
      {
        productId: perishables[2].product.id,
        toLocationId: location.id,
        qty: perishables[2].qty,
        type: "RECEIPT",
        reason: "Bakery delivery",
        batchId: batches[2].id,
        ebmSyncState: "ACKNOWLEDGED",
        orgId,
      },
    ],
  });

  // ---- promotions and price history --------------------------------------
  const promoMilk = item("Inyange UHT milk");
  const promoSoap = item("Omo washing powder");
  await db.promotion.createMany({
    data: [
      {
        productId: promoMilk.id,
        price: money(1300).toFixed(2),
        startsAt: new Date(now.getTime() - DAY_MS),
        endsAt: new Date(now.getTime() + 5 * DAY_MS),
        orgId,
      },
      {
        productId: promoSoap.id,
        price: money(2990).toFixed(2),
        startsAt: new Date(now.getTime() + 7 * DAY_MS),
        endsAt: new Date(now.getTime() + 14 * DAY_MS),
        orgId,
      },
    ],
  });
  const sugar = item("Kabuye white sugar");
  const oil = item("Sunflower cooking oil");
  await db.priceChange.createMany({
    data: [
      {
        productId: sugar.id,
        oldPrice: money(1700).toFixed(2),
        newPrice: money(sugar.unitPrice).toFixed(2),
        changedAt: new Date(now.getTime() - 9 * DAY_MS),
        changedById: cashier.id,
        orgId,
      },
      {
        productId: oil.id,
        oldPrice: money(2600).toFixed(2),
        newPrice: money(oil.unitPrice).toFixed(2),
        changedAt: new Date(now.getTime() - 3 * DAY_MS),
        changedById: cashier.id,
        orgId,
      },
    ],
  });

  // ---- lanes --------------------------------------------------------------
  // mrc is left null: whether the RRA machine certificate is per org, per lane or per
  // install is still an open question, so the seed does not invent one.
  const laneNames = ["Till 1", "Till 2", "Till 3 — Express"];
  const lanes = [];
  for (let i = 0; i < laneNames.length; i++) {
    lanes.push(
      await db.lane.create({
        data: {
          code: `T${i + 1}`,
          name: laneNames[i],
          locationId: location.id,
          bhfId: "00",
          orgId,
        },
      }),
    );
  }

  // ---- yesterday's trading day, then today's open shift -------------------
  const yesterday = new Date(now.getTime() - DAY_MS);
  for (let i = 0; i < lanes.length; i++) {
    const lane = lanes[i];
    const shift = await db.shift.create({
      data: {
        laneId: lane.id,
        openedById: cashier.id,
        openedAt: atHour(yesterday, 8),
        openingFloat: money(20_000).toFixed(2),
        orgId,
      },
    });
    const cash = await ringBaskets({
      db,
      orgId,
      rng,
      products,
      laneId: lane.id,
      shiftId: shift.id,
      firstInvcNo: 1,
      count: SALES_PER_LANE,
      from: atHour(yesterday, 8, 25),
      everyMinutes: 33,
      // One basket on Till 2 is voided after payment, so the lane screens have a void.
      voidAt: i === 1 ? 13 : null,
    });
    // Declared cash is the float plus the day's cash takings, with the small over or
    // short a real till ends on.
    const variance = [-500, 0, 200][i];
    await db.shift.update({
      where: { id: shift.id },
      data: {
        closedAt: atHour(yesterday, 20, 45),
        declaredCash: money(20_000).plus(cash).plus(variance).toFixed(2),
      },
    });
    await db.lane.update({ where: { id: lane.id }, data: { lastInvcNo: SALES_PER_LANE } });
  }

  // Today's shift on Till 1 is still open, with the morning's baskets on it.
  const openShift = await db.shift.create({
    data: {
      laneId: lanes[0].id,
      openedById: cashier.id,
      openedAt: new Date(now.getTime() - 2 * 3_600_000),
      openingFloat: money(20_000).toFixed(2),
      orgId,
    },
  });
  await ringBaskets({
    db,
    orgId,
    rng,
    products,
    laneId: lanes[0].id,
    shiftId: openShift.id,
    firstInvcNo: SALES_PER_LANE + 1,
    count: SALES_TODAY,
    from: new Date(now.getTime() - 95 * 60_000),
    everyMinutes: 22,
    voidAt: null,
  });
  await db.lane.update({
    where: { id: lanes[0].id },
    data: { lastInvcNo: SALES_PER_LANE + SALES_TODAY },
  });
}

// ===========================================================================
// Baskets
// ===========================================================================

/** Yesterday's trading day, per lane, plus this morning's baskets on the open shift. */
const SALES_PER_LANE = 21;
const SALES_TODAY = 5;

/** How often each department turns up in a basket. */
const BASKET_WEIGHTS: Record<string, number> = {
  "Fresh Produce": 22,
  Butchery: 10,
  Dairy: 16,
  Bakery: 14,
  "Dry Goods": 18,
  Beverages: 12,
  Household: 5,
  "Personal Care": 3,
};

const LINE_COUNTS = [1, 1, 2, 2, 3, 3, 3, 4, 4, 5, 6, 8];

type DraftLine = {
  product: SeededProduct;
  qty: Prisma.Decimal;
  discountKind: "PERCENT" | "AMOUNT" | null;
  discountValue: Prisma.Decimal | null;
};

/**
 * Rings `count` baskets onto one lane and shift, and returns the cash they took
 * (voided baskets excluded, their cash having gone back over the counter).
 */
async function ringBaskets(opts: {
  db: SeedClient;
  orgId: string;
  rng: () => number;
  products: SeededProduct[];
  laneId: string;
  shiftId: string;
  firstInvcNo: number;
  count: number;
  from: Date;
  everyMinutes: number;
  /** 1-based index of the basket to void, if any. */
  voidAt: number | null;
}): Promise<Prisma.Decimal> {
  const { db, orgId, rng, products, laneId, shiftId, count, from, everyMinutes, voidAt } = opts;
  const weighed = products.filter((p) => p.isWeighed);
  let cash = new Prisma.Decimal(0);

  for (let n = 0; n < count; n++) {
    const invcNo = opts.firstInvcNo + n;
    const soldAt = new Date(from.getTime() + n * everyMinutes * 60_000 + Math.floor(rng() * 9) * 60_000);
    const lines = buildLines(rng, products, weighed);
    const totals = priceBasket(lines, invcNo);
    const tenders = tenderFor(rng, totals.totAmt, invcNo);
    const voided = voidAt !== null && n + 1 === voidAt;

    await db.sale.create({
      data: {
        shiftId,
        laneId,
        invcNo,
        rcptNo: invcNo,
        totAmt: totals.totAmt.toFixed(2),
        totTaxblAmt: totals.totTaxblAmt.toFixed(2),
        totTaxAmt: totals.totTaxAmt.toFixed(2),
        // Every so often a shopper wants the EBM invoice in their company's name.
        buyerTin: invcNo % 11 === 4 ? `10${String(2_345_600 + invcNo)}` : null,
        discountKind: totals.discountKind,
        discountValue: totals.discountValue?.toFixed(2) ?? null,
        voidedAt: voided ? new Date(soldAt.getTime() + 120_000) : null,
        soldAt,
        orgId,
        lines: {
          create: lines.map((l, i) => ({
            productId: l.product.id,
            itemSeq: i + 1,
            qty: l.qty.toFixed(3),
            prc: l.product.unitPrice.toFixed(2),
            taxTyCd: l.product.taxTyCd,
            discountKind: l.discountKind,
            discountValue: l.discountValue?.toFixed(2) ?? null,
            orgId,
          })),
        },
        // No providerRef, no confirmed: mobile money is recorded, not confirmed, in v1.
        tenders: {
          create: tenders.map((t) => ({
            pmtTyCd: t.method,
            amount: t.amount.toFixed(2),
            orgId,
          })),
        },
      },
    });

    if (!voided) {
      for (const t of tenders) if (t.method === "CASH") cash = cash.plus(t.amount);
    }
  }
  return cash;
}

function buildLines(
  rng: () => number,
  products: SeededProduct[],
  weighed: SeededProduct[],
): DraftLine[] {
  const want = LINE_COUNTS[Math.floor(rng() * LINE_COUNTS.length)];
  const chosen: SeededProduct[] = [];
  const seen = new Set<string>();
  // Nearly half the baskets start at the scale, so weighed lines are well represented.
  if (rng() < 0.45) {
    const p = weighed[Math.floor(rng() * weighed.length)];
    chosen.push(p);
    seen.add(p.id);
  }
  let guard = 0;
  while (chosen.length < want && guard++ < 40) {
    const p = pickProduct(rng, products);
    if (seen.has(p.id)) continue;
    seen.add(p.id);
    chosen.push(p);
  }

  return chosen.map((product) => {
    const qty = product.isWeighed ? weight(rng) : units(rng);
    const perishable = !product.isWeighed && ["Bakery", "Dairy"].includes(product.department);
    const roll = rng();
    // The odd perishable goes out at a close-of-day markdown; a big line now and then
    // carries a flat shelf-edge offer. The floor keeps a discount well under the line.
    if (perishable && roll < 0.08) {
      return { product, qty, discountKind: "PERCENT" as const, discountValue: new Prisma.Decimal(10) };
    }
    if (roll > 0.95 && lineAmount(qty, product.unitPrice).gt(5_000)) {
      return { product, qty, discountKind: "AMOUNT" as const, discountValue: new Prisma.Decimal(500) };
    }
    return { product, qty, discountKind: null, discountValue: null };
  });
}

function pickProduct(rng: () => number, products: SeededProduct[]): SeededProduct {
  const total = Object.values(BASKET_WEIGHTS).reduce((a, b) => a + b, 0);
  let roll = rng() * total;
  let dept = "Dry Goods";
  for (const [name, weight] of Object.entries(BASKET_WEIGHTS)) {
    roll -= weight;
    if (roll <= 0) {
      dept = name;
      break;
    }
  }
  const pool = products.filter((p) => p.department === dept);
  return pool[Math.floor(rng() * pool.length)];
}

/** A weight off the scale: 0.15 to 3.5 kg at three decimals, never a whole number. */
function weight(rng: () => number): Prisma.Decimal {
  const kg = toQty(0.15 + rng() * 3.35).toDecimalPlaces(3);
  return kg.isInteger() ? kg.plus("0.125") : kg;
}

/** Units of a packaged line: usually one, sometimes a few. */
function units(rng: () => number): Prisma.Decimal {
  const r = rng();
  return toQty(r < 0.68 ? 1 : r < 0.9 ? 2 : r < 0.97 ? 3 : 6);
}

/**
 * Prices a basket the way an EBM receipt reads: a VAT-inclusive total, the base it sits
 * on and the tax carved out of it, with totAmt === totTaxblAmt + totTaxAmt to the cent.
 * A basket discount is prorated between the exempt and standard-rated halves so the
 * carved-out tax still matches what was actually charged.
 */
function priceBasket(lines: DraftLine[], invcNo: number) {
  let exemptNet = new Prisma.Decimal(0);
  let standardNet = new Prisma.Decimal(0);
  for (const l of lines) {
    const gross = lineAmount(l.qty, l.product.unitPrice);
    const off =
      l.discountKind === "PERCENT" && l.discountValue
        ? gross.times(l.discountValue).div(100).toDecimalPlaces(2)
        : l.discountKind === "AMOUNT" && l.discountValue
          ? Prisma.Decimal.min(l.discountValue, gross)
          : new Prisma.Decimal(0);
    const net = gross.minus(off);
    if (l.product.taxTyCd === "A") exemptNet = exemptNet.plus(net);
    else standardNet = standardNet.plus(net);
  }
  const subtotal = exemptNet.plus(standardNet);

  // Every 17th basket carries a basket-level discount, alternating the two kinds.
  let discountKind: "PERCENT" | "AMOUNT" | null = null;
  let discountValue: Prisma.Decimal | null = null;
  let off = new Prisma.Decimal(0);
  if (invcNo % 17 === 3 && subtotal.gt(5_000)) {
    if (invcNo % 34 === 3) {
      discountKind = "PERCENT";
      discountValue = new Prisma.Decimal(5);
      off = subtotal.times(5).div(100).toDecimalPlaces(2);
    } else {
      discountKind = "AMOUNT";
      discountValue = new Prisma.Decimal(500);
      off = new Prisma.Decimal(500);
    }
  }

  const totAmt = subtotal.minus(off);
  const standardOff = subtotal.isZero()
    ? new Prisma.Decimal(0)
    : standardNet.div(subtotal).times(off).toDecimalPlaces(2);
  const totTaxAmt = vatOf(standardNet.minus(standardOff));
  return { totAmt, totTaxAmt, totTaxblAmt: totAmt.minus(totTaxAmt), discountKind, discountValue };
}

/**
 * How the basket was paid. Mostly cash, a good share of mobile money, some cards, and
 * every seventh basket split between cash and mobile money — the case Phase 2's lane has
 * to handle and the reason this seed exists.
 */
function tenderFor(
  rng: () => number,
  totAmt: Prisma.Decimal,
  invcNo: number,
): { method: "CASH" | "MOBILE_MONEY" | "CARD"; amount: Prisma.Decimal }[] {
  if (invcNo % 7 === 0) {
    // The shopper sends a round amount off MoMo and pays the rest in cash. On a small
    // basket there is no round amount to send, so it splits down the middle instead.
    const mobile = totAmt.gte(2_000)
      ? Prisma.Decimal.min(
          Prisma.Decimal.max(1_000, totAmt.times(0.6).div(500).floor().times(500)),
          totAmt.minus(100),
        )
      : totAmt.div(2).toDecimalPlaces(2);
    return [
      { method: "MOBILE_MONEY", amount: mobile },
      { method: "CASH", amount: totAmt.minus(mobile) },
    ];
  }
  const r = rng();
  const method = r < 0.55 ? "CASH" : r < 0.85 ? "MOBILE_MONEY" : "CARD";
  return [{ method, amount: totAmt }];
}

// ===========================================================================
// Direct run: a dedicated grocery org, so the legacy B2B catalogue is untouched.
// ===========================================================================

const ORG_SLUG = "market";
const ADMIN_EMAIL = "admin@market.test";

async function main() {
  const db = defaultDb();
  try {
    console.log("Seeding a Rwandan supermarket...");
    const org = await db.org.upsert({
      where: { slug: ORG_SLUG },
      update: {},
      create: {
        slug: ORG_SLUG,
        name: "Nyabugogo Market",
        currency: "RWF",
        timezone: "Africa/Kigali",
      },
    });
    await db.user.upsert({
      where: { email_orgId: { email: ADMIN_EMAIL, orgId: org.id } },
      update: {},
      create: {
        email: ADMIN_EMAIL,
        password: await bcrypt.hash("password123", 10),
        name: "Market Manager",
        role: "ADMIN",
        orgId: org.id,
      },
    });

    // Additive only, and never twice: a second pass would collide on sku and barcode.
    const already = await db.product.count({
      where: { orgId: org.id, department: { not: "General" } },
    });
    if (already > 0) {
      console.log(`  ${org.slug} already holds ${already} grocery products; nothing to do.`);
      return;
    }

    await seedGrocery(org.id, db);

    const where = { orgId: org.id };
    const [products, weighedCount, exempt, barcoded, lanes, sales] = await Promise.all([
      db.product.count({ where: { ...where, department: { not: "General" } } }),
      db.product.count({ where: { ...where, isWeighed: true } }),
      db.product.count({ where: { ...where, taxTyCd: "A" } }),
      db.product.count({ where: { ...where, barcode: { not: null } } }),
      db.lane.count({ where }),
      db.sale.count({ where }),
    ]);
    console.log(
      `  ${products} products (${weighedCount} weighed, ${exempt} VAT-exempt, ${barcoded} barcoded)`,
    );
    console.log(`  ${lanes} lanes, ${sales} sales`);
    console.log("\nSeed complete!");
    console.log(`Shop admin:  ${ADMIN_EMAIL} / password123     ->  /${ORG_SLUG}/dashboard`);
  } finally {
    await db.$disconnect();
  }
}

// Only when run as a script: importing this module (as the tests do) must not seed.
if (process.argv[1] && basename(process.argv[1]).startsWith("seed-grocery")) {
  void main().catch((e) => {
    console.error(e);
    process.exitCode = 1;
  });
}
