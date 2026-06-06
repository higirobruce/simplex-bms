import { PrismaClient } from "@prisma/client";
import bcrypt from "bcryptjs";

const prisma = new PrismaClient();

async function main() {
  console.log("Seeding database...");

  // Platform super admin (belongs to no shop)
  const superHashed = await bcrypt.hash("password123", 10);
  const superAdmin = await prisma.user.create({
    data: {
      email: "super@simplex.test",
      password: superHashed,
      name: "Platform Admin",
      role: "ADMIN",
      isSuperAdmin: true,
    },
  });
  console.log("Created super admin:", superAdmin.email);

  // Platform settings singleton
  await prisma.platformSettings.upsert({
    where: { id: "platform" },
    update: {},
    create: { id: "platform", platformName: "Simplex" },
  });

  // Create test org
  const org = await prisma.org.create({
    data: {
      slug: "acme",
      name: "ACME Corporation",
      currency: "RWF",
      timezone: "Africa/Kigali",
    },
  });
  console.log("Created org:", org.name);

  // Create admin user
  const hashedPassword = await bcrypt.hash("password123", 10);
  const admin = await prisma.user.create({
    data: {
      email: "admin@acme.test",
      password: hashedPassword,
      name: "Admin User",
      role: "ADMIN",
      orgId: org.id,
    },
  });
  console.log("Created user:", admin.email);

  // Create warehouse hierarchy: Main → (Showroom, Storage)
  const warehouse = await prisma.location.create({
    data: {
      name: "Main Warehouse",
      code: "WH-01",
      address: "123 Kigali Ave",
      orgId: org.id,
    },
  });
  const showroom = await prisma.location.create({
    data: {
      name: "Showroom",
      code: "WH-01-A",
      parentId: warehouse.id,
      orgId: org.id,
    },
  });
  const storage = await prisma.location.create({
    data: {
      name: "Back Storage",
      code: "WH-01-B",
      parentId: warehouse.id,
      orgId: org.id,
    },
  });
  const branch = await prisma.location.create({
    data: {
      name: "Nyarutarama Branch",
      code: "WH-02",
      address: "Nyarutarama, Kigali",
      orgId: org.id,
    },
  });
  console.log("Created warehouse hierarchy");

  // Create products
  const products = await Promise.all([
    prisma.product.create({
      data: {
        sku: "LAP-001",
        name: "Laptop HP ProBook",
        category: "Electronics",
        unitPrice: 450000,
        costPrice: 380000,
        reorderLevel: 5,
        orgId: org.id,
      },
    }),
    prisma.product.create({
      data: {
        sku: "PHN-001",
        name: "Samsung Galaxy A54",
        category: "Electronics",
        unitPrice: 250000,
        costPrice: 200000,
        reorderLevel: 10,
        orgId: org.id,
      },
    }),
    prisma.product.create({
      data: {
        sku: "DSK-001",
        name: "Office Desk",
        category: "Furniture",
        unitPrice: 85000,
        costPrice: 60000,
        reorderLevel: 3,
        orgId: org.id,
      },
    }),
    prisma.product.create({
      data: {
        sku: "CHR-001",
        name: "Ergonomic Chair",
        category: "Furniture",
        unitPrice: 120000,
        costPrice: 90000,
        reorderLevel: 5,
        orgId: org.id,
      },
    }),
    prisma.product.create({
      data: {
        sku: "PRT-001",
        name: "HP LaserJet Printer",
        category: "Electronics",
        unitPrice: 320000,
        costPrice: 260000,
        reorderLevel: 2,
        orgId: org.id,
      },
    }),
  ]);
  console.log(`Created ${products.length} products`);

  // Create stock levels — split between sub-locations and the branch
  const stockPlan = [
    { qtyShowroom: 5, qtyStorage: 8, qtyBranch: 2 },   // LAP-001
    { qtyShowroom: 8, qtyStorage: 12, qtyBranch: 5 },  // PHN-001
    { qtyShowroom: 2, qtyStorage: 4, qtyBranch: 2 },   // DSK-001
    { qtyShowroom: 4, qtyStorage: 6, qtyBranch: 2 },   // CHR-001
    { qtyShowroom: 1, qtyStorage: 1, qtyBranch: 1 },   // PRT-001
  ];
  for (let i = 0; i < products.length; i++) {
    const plan = stockPlan[i];
    await prisma.stockLevel.create({
      data: { productId: products[i].id, locationId: showroom.id, qty: plan.qtyShowroom },
    });
    await prisma.stockLevel.create({
      data: { productId: products[i].id, locationId: storage.id, qty: plan.qtyStorage },
    });
    await prisma.stockLevel.create({
      data: { productId: products[i].id, locationId: branch.id, qty: plan.qtyBranch },
    });
  }
  console.log("Created stock levels across warehouses");

  // Create customers
  const customers = await Promise.all([
    prisma.customer.create({
      data: {
        name: "Kigali Tech Solutions",
        email: "info@kigalitech.rw",
        phone: "+250 788 123 456",
        city: "Kigali",
        country: "Rwanda",
        orgId: org.id,
      },
    }),
    prisma.customer.create({
      data: {
        name: "Rwanda Business Hub",
        email: "contact@rwbizhub.rw",
        phone: "+250 788 234 567",
        city: "Kigali",
        country: "Rwanda",
        orgId: org.id,
      },
    }),
    prisma.customer.create({
      data: {
        name: "East Africa Traders",
        email: "sales@eatraders.com",
        phone: "+250 788 345 678",
        city: "Kigali",
        country: "Rwanda",
        orgId: org.id,
      },
    }),
  ]);
  console.log(`Created ${customers.length} customers`);

  // Create vendors
  const vendors = await Promise.all([
    prisma.vendor.create({
      data: {
        name: "HP Rwanda Distributors",
        email: "supply@hprw.com",
        phone: "+250 788 456 789",
        paymentTerms: "NET30",
        orgId: org.id,
      },
    }),
    prisma.vendor.create({
      data: {
        name: "Samsung East Africa",
        email: "orders@samsung-ea.com",
        phone: "+250 788 567 890",
        paymentTerms: "NET60",
        orgId: org.id,
      },
    }),
  ]);
  console.log("Created vendors");

  // Create invoices
  const inv1 = await prisma.invoice.create({
    data: {
      invoiceNo: "INV-0001",
      customerId: customers[0].id,
      status: "PAID",
      subtotal: 700000,
      total: 700000,
      amountPaid: 700000,
      dueDate: new Date("2026-04-30"),
      orgId: org.id,
      lineItems: {
        create: [
          { productId: products[0].id, qty: 1, unitPrice: 450000, amount: 450000 },
          { productId: products[1].id, qty: 1, unitPrice: 250000, amount: 250000 },
        ],
      },
    },
  });

  await prisma.payment.create({
    data: {
      invoiceId: inv1.id,
      amount: 700000,
      method: "BANK",
      date: new Date("2026-04-05"),
    },
  });

  const inv2 = await prisma.invoice.create({
    data: {
      invoiceNo: "INV-0002",
      customerId: customers[1].id,
      status: "SENT",
      subtotal: 205000,
      total: 205000,
      amountPaid: 0,
      dueDate: new Date("2026-04-25"),
      orgId: org.id,
      lineItems: {
        create: [
          { productId: products[2].id, qty: 1, unitPrice: 85000, amount: 85000 },
          { productId: products[3].id, qty: 1, unitPrice: 120000, amount: 120000 },
        ],
      },
    },
  });

  const inv3 = await prisma.invoice.create({
    data: {
      invoiceNo: "INV-0003",
      customerId: customers[2].id,
      status: "PARTIAL",
      subtotal: 320000,
      total: 320000,
      amountPaid: 150000,
      dueDate: new Date("2026-04-20"),
      orgId: org.id,
      lineItems: {
        create: [
          { productId: products[4].id, qty: 1, unitPrice: 320000, amount: 320000 },
        ],
      },
    },
  });

  await prisma.payment.create({
    data: {
      invoiceId: inv3.id,
      amount: 150000,
      method: "MOBILE_MONEY",
      date: new Date("2026-04-08"),
    },
  });

  console.log("Created 3 invoices with payments");

  // Create expenses
  await Promise.all([
    prisma.expense.create({
      data: { category: "rent", amount: 200000, date: new Date("2026-04-01"), description: "Monthly office rent", orgId: org.id },
    }),
    prisma.expense.create({
      data: { category: "utilities", amount: 35000, date: new Date("2026-04-03"), description: "Electricity bill", orgId: org.id },
    }),
    prisma.expense.create({
      data: { category: "salary", amount: 500000, date: new Date("2026-04-05"), description: "Staff salaries", orgId: org.id },
    }),
  ]);
  console.log("Created expenses");

  // Sample sales order (DRAFT) — ready for the user to confirm + deliver
  await prisma.salesOrder.create({
    data: {
      orgId: org.id,
      orderNo: "SO-0001",
      customerId: customers[0].id,
      status: "DRAFT",
      subtotal: 700000,
      total: 700000,
      createdById: admin.id,
      lines: {
        create: [
          { productId: products[0].id, qty: 1, unitPrice: 450000, amount: 450000 },
          { productId: products[1].id, qty: 1, unitPrice: 250000, amount: 250000 },
        ],
      },
    },
  });

  // Sample purchase order (DRAFT)
  await prisma.purchaseOrder.create({
    data: {
      orgId: org.id,
      orderNo: "PO-0001",
      vendorId: vendors[0].id,
      status: "DRAFT",
      subtotal: 760000,
      total: 760000,
      createdById: admin.id,
      lines: {
        create: [
          { productId: products[0].id, qty: 2, unitPrice: 380000, amount: 760000 },
        ],
      },
    },
  });
  console.log("Created sample sales + purchase orders");

  // ===== Bulk demo data so every list spans multiple pages =====
  const cats = ["Electronics", "Furniture", "Tools", "Hardware", "Office"];
  const itemNames = [
    "Cordless Drill", "Steel Hammer", "Paint Roller", "LED Bulb Pack",
    "Extension Cable", "Wall Bracket", "Tool Box", "Safety Gloves",
    "Measuring Tape", "Pipe Wrench", "Screwdriver Set", "Work Bench",
    "Angle Grinder", "Ladder 3m", "Hose Reel", "Padlock", "Door Hinge",
    "Caulk Gun", "Sandpaper Pack", "Utility Knife", "Welding Mask",
    "Air Compressor", "Nail Gun", "Socket Set", "Spirit Level",
    "Hex Key Set", "Bolt Cutter", "Stud Finder", "Heat Gun", "Putty Knife",
  ];
  const extraProducts = [];
  for (let i = 0; i < itemNames.length; i++) {
    const unitPrice = 5000 + ((i * 3137) % 90) * 1000;
    const p = await prisma.product.create({
      data: {
        sku: `HW-${String(i + 1).padStart(3, "0")}`,
        name: itemNames[i],
        category: cats[i % cats.length],
        unitPrice,
        costPrice: Math.round(unitPrice * 0.7),
        reorderLevel: 5 + (i % 6),
        orgId: org.id,
      },
    });
    extraProducts.push(p);
    await prisma.stockLevel.create({ data: { productId: p.id, locationId: storage.id, qty: (i * 7) % 40 } });
    await prisma.stockLevel.create({ data: { productId: p.id, locationId: showroom.id, qty: (i * 3) % 15 } });
  }
  const allProducts = [...products, ...extraProducts];

  const extraCustomers = [];
  for (let i = 1; i <= 30; i++) {
    extraCustomers.push(
      await prisma.customer.create({
        data: {
          name: `Demo Customer ${String(i).padStart(2, "0")}`,
          email: `customer${i}@demo.rw`,
          phone: `+250 788 ${100000 + i}`,
          city: "Kigali",
          country: "Rwanda",
          creditLimit: (i % 5) * 100000,
          orgId: org.id,
        },
      })
    );
  }
  const allCustomers = [...customers, ...extraCustomers];

  const terms = ["NET30", "NET60", "NET15", "COD"];
  const extraVendors = [];
  for (let i = 1; i <= 25; i++) {
    extraVendors.push(
      await prisma.vendor.create({
        data: {
          name: `Demo Vendor ${String(i).padStart(2, "0")}`,
          email: `vendor${i}@supply.rw`,
          phone: `+250 789 ${100000 + i}`,
          paymentTerms: terms[i % terms.length],
          orgId: org.id,
        },
      })
    );
  }
  const allVendors = [...vendors, ...extraVendors];

  const expCats = ["rent", "utilities", "salary", "supplies", "transport", "marketing", "other"];
  for (let i = 1; i <= 30; i++) {
    await prisma.expense.create({
      data: {
        category: expCats[i % expCats.length],
        amount: 10000 + ((i * 7919) % 500000),
        date: new Date(2026, 4, (i % 28) + 1),
        description: `Expense entry ${i}`,
        orgId: org.id,
      },
    });
  }

  const invStatuses = ["DRAFT", "SENT", "PARTIAL", "PAID", "OVERDUE"] as const;
  for (let i = 4; i <= 33; i++) {
    const cust = allCustomers[i % allCustomers.length];
    const pa = allProducts[i % allProducts.length];
    const pb = allProducts[(i + 3) % allProducts.length];
    const qa = 1 + (i % 3);
    const qb = 1 + ((i + 1) % 2);
    const amtA = Number(pa.unitPrice) * qa;
    const amtB = Number(pb.unitPrice) * qb;
    const subtotal = amtA + amtB;
    const status = invStatuses[i % invStatuses.length];
    const amountPaid = status === "PAID" ? subtotal : status === "PARTIAL" ? Math.round(subtotal / 2) : 0;
    await prisma.invoice.create({
      data: {
        invoiceNo: `INV-${String(i).padStart(4, "0")}`,
        customerId: cust.id,
        status,
        subtotal,
        total: subtotal,
        amountPaid,
        dueDate: new Date(2026, 4, (i % 28) + 1),
        orgId: org.id,
        lineItems: {
          create: [
            { productId: pa.id, qty: qa, unitPrice: pa.unitPrice, amount: amtA },
            { productId: pb.id, qty: qb, unitPrice: pb.unitPrice, amount: amtB },
          ],
        },
      },
    });
  }

  const soStatuses = ["DRAFT", "CONFIRMED", "FULFILLED", "CANCELLED"] as const;
  for (let i = 2; i <= 31; i++) {
    const cust = allCustomers[i % allCustomers.length];
    const prod = allProducts[i % allProducts.length];
    const qty = 1 + (i % 4);
    const amt = Number(prod.unitPrice) * qty;
    await prisma.salesOrder.create({
      data: {
        orgId: org.id,
        orderNo: `SO-${String(i).padStart(4, "0")}`,
        customerId: cust.id,
        status: soStatuses[i % soStatuses.length],
        subtotal: amt,
        total: amt,
        createdById: admin.id,
        lines: { create: [{ productId: prod.id, qty, unitPrice: prod.unitPrice, amount: amt }] },
      },
    });
  }

  const poStatuses = ["DRAFT", "CONFIRMED", "RECEIVED", "CANCELLED"] as const;
  for (let i = 2; i <= 31; i++) {
    const vend = allVendors[i % allVendors.length];
    const prod = allProducts[i % allProducts.length];
    const qty = 2 + (i % 5);
    const price = Number(prod.costPrice ?? prod.unitPrice);
    const amt = price * qty;
    await prisma.purchaseOrder.create({
      data: {
        orgId: org.id,
        orderNo: `PO-${String(i).padStart(4, "0")}`,
        vendorId: vend.id,
        status: poStatuses[i % poStatuses.length],
        subtotal: amt,
        total: amt,
        createdById: admin.id,
        lines: { create: [{ productId: prod.id, qty, unitPrice: price, amount: amt }] },
      },
    });
  }
  console.log("Created bulk demo data (35 products, 33 customers, 27 vendors, 33 invoices, 31 SOs, 31 POs, 33 expenses)");

  console.log("\nSeed complete!");
  console.log("Super admin: super@simplex.test / password123  →  /admin");
  console.log("Shop admin:  admin@acme.test / password123     →  /acme/dashboard");
}

main()
  .catch(console.error)
  .finally(() => prisma.$disconnect());
