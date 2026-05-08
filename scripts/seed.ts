import { PrismaClient } from "@prisma/client";
import bcrypt from "bcryptjs";

const prisma = new PrismaClient();

async function main() {
  console.log("Seeding database...");

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

  // Create location
  const warehouse = await prisma.location.create({
    data: {
      name: "Main Warehouse",
      address: "123 Kigali Ave",
      orgId: org.id,
    },
  });

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

  // Create stock levels
  await Promise.all(
    products.map((p, i) =>
      prisma.stockLevel.create({
        data: {
          productId: p.id,
          locationId: warehouse.id,
          qty: [15, 25, 8, 12, 3][i],
        },
      })
    )
  );
  console.log("Created stock levels");

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
  await Promise.all([
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

  console.log("\nSeed complete!");
  console.log("Login with: admin@acme.test / password123");
  console.log("Org slug: acme");
}

main()
  .catch(console.error)
  .finally(() => prisma.$disconnect());
