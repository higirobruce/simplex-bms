import { beforeEach, describe, expect, test, vi } from "vitest";
import { prisma, seedOrg } from "./helpers/db";

vi.mock("@/lib/prisma", async () => ({ prisma: (await import("./helpers/db")).prisma }));
const getServerSession = vi.hoisted(() => vi.fn());
vi.mock("next-auth", () => ({ getServerSession }));

import argon2 from "argon2";
import bcrypt from "bcryptjs";
import { POST, GET, PATCH } from "@/app/api/[slug]/cashiers/route";
import { authOptions } from "@/lib/auth";

const as = (userId: string, orgId: string, role = "ADMIN") =>
  getServerSession.mockResolvedValue({ user: { id: userId, orgId, role } });
const url = "http://x.local/api/shop/cashiers";
const req = (method: string, body?: unknown) => new Request(url, { method, body: body ? JSON.stringify(body) : undefined });
const post = (body: unknown) => POST(req("POST", body));
const authorize = (email: string, password: string) =>
  (authOptions.providers[0] as any).options.authorize({ email, password });

beforeEach(() => getServerSession.mockReset());

describe("cashier accounts", () => {
  test("a PIN shorter than 6 digits is refused", async () => {
    const s = await seedOrg();
    as(s.userId, s.orgId);
    expect((await post({ name: "Amy", pin: "12345" })).status).toBe(400);
    expect(await prisma.user.count({ where: { role: "CASHIER" } })).toBe(0);
  });

  test("a non-numeric PIN is refused", async () => {
    const s = await seedOrg();
    as(s.userId, s.orgId);
    for (const pin of ["12345a", "abcdef", "123 456", "12.456", 123456, null]) {
      expect((await post({ name: "Amy", pin })).status).toBe(400);
    }
    expect(await prisma.user.count({ where: { role: "CASHIER" } })).toBe(0);
  });

  test("the stored pinHash is argon2id and is not the password column", async () => {
    const s = await seedOrg();
    as(s.userId, s.orgId);
    const res = await post({ name: "Amy", pin: "123456" });
    expect(res.status).toBe(201);
    const text = JSON.stringify(await res.json());
    const row = await prisma.user.findFirstOrThrow({ where: { role: "CASHIER" } });
    expect(row.pinHash).toMatch(/^\$argon2id\$v=19\$m=\d+,t=\d+,p=\d+\$/);
    expect(Number(row.pinHash!.match(/m=(\d+)/)![1])).toBeGreaterThanOrEqual(65536);
    expect(await argon2.verify(row.pinHash!, "123456")).toBe(true);
    expect(await argon2.verify(row.pinHash!, "654321")).toBe(false);
    expect(row.password).not.toBe(row.pinHash);
    expect(row.password).not.toContain("123456");
    expect(row.orgId).toBe(s.orgId);
    expect(text).not.toContain("123456");
    expect(text).not.toContain(row.pinHash!);
    const audits = await prisma.auditLog.findMany({ where: { entityId: row.id } });
    expect(audits).toHaveLength(1);
    expect(JSON.stringify(audits)).not.toContain("123456");
    expect(JSON.stringify(audits)).not.toContain(row.pinHash!);
  });

  test("a cashier cannot log in to the web app with their PIN", async () => {
    const s = await seedOrg();
    as(s.userId, s.orgId);
    await post({ name: "Amy", pin: "123456" });
    const row = await prisma.user.findFirstOrThrow({ where: { role: "CASHIER" } });
    // credentials path, with the PIN, an empty-ish password, guesses and the hash itself
    for (const pw of ["123456", "x", "!", row.password, row.pinHash!, "cashier-no-web-login", " "]) {
      expect(await authorize(row.email, pw)).toBeNull();
      expect(await bcrypt.compare(pw, row.password)).toBe(false);
    }
    // sanity: the same path does admit a real web user, so null above is meaningful
    const hash = await bcrypt.hash("real-pass", 4);
    await prisma.user.create({ data: { email: "web@t.local", password: hash, role: "ADMIN", orgId: s.orgId } });
    expect(await authorize("web@t.local", "real-pass")).toMatchObject({ email: "web@t.local" });
    expect(await authorize("web@t.local", "wrong")).toBeNull();
  });

  test("two cashiers with the same name in one org get different emails", async () => {
    const s = await seedOrg();
    as(s.userId, s.orgId);
    expect((await post({ name: "Amy", pin: "123456" })).status).toBe(201);
    expect((await post({ name: "Amy", pin: "123456" })).status).toBe(201);
    const rows = await prisma.user.findMany({ where: { role: "CASHIER" } });
    expect(rows).toHaveLength(2);
    expect(rows[0].email).not.toBe(rows[1].email);
  });

  test("the same name in two orgs is allowed", async () => {
    const s = await seedOrg();
    const s2 = await seedOrg();
    as(s.userId, s.orgId);
    expect((await post({ name: "Amy", pin: "123456" })).status).toBe(201);
    as(s2.userId, s2.orgId);
    expect((await post({ name: "Amy", pin: "123456" })).status).toBe(201);
    const rows = await prisma.user.findMany({ where: { role: "CASHIER" } });
    expect(new Set(rows.map((r) => r.orgId)).size).toBe(2);
  });

  test("only an ADMIN can manage cashiers; tenant comes from the session", async () => {
    const s = await seedOrg();
    for (const role of ["MANAGER", "ACCOUNTANT", "VIEWER", "CASHIER"]) {
      as(s.userId, s.orgId, role);
      expect((await post({ name: "Amy", pin: "123456" })).status).toBe(403);
      expect((await GET()).status).toBe(403);
    }
    as(s.userId, s.orgId);
    await post({ name: "Amy", pin: "123456", orgId: s.otherOrgId });
    expect((await prisma.user.findFirstOrThrow({ where: { role: "CASHIER" } })).orgId).toBe(s.orgId);
    getServerSession.mockResolvedValue(null);
    expect((await post({ name: "Amy", pin: "123456" })).status).toBe(401);
  });

  test("list, set a new PIN, deactivate; other orgs' cashiers are untouchable", async () => {
    const s = await seedOrg();
    as(s.userId, s.orgId);
    const { id } = await (await post({ name: "Amy", pin: "123456" })).json();
    const foreign = await prisma.user.create({
      data: { email: "f@c.invalid", password: "!x", role: "CASHIER", orgId: s.otherOrgId, pinHash: "h" },
    });
    const text = await (await GET()).text();
    const { data } = JSON.parse(text);
    expect(data).toHaveLength(1);
    expect(data[0]).toMatchObject({ id, name: "Amy", active: true });
    expect(text).not.toContain("argon2");
    expect(text).not.toContain("@");

    const before = (await prisma.user.findUniqueOrThrow({ where: { id } })).pinHash!;
    expect((await PATCH(req("PATCH", { id, pin: "99" }))).status).toBe(400);
    expect((await PATCH(req("PATCH", { id, pin: "987654" }))).status).toBe(200);
    const after = (await prisma.user.findUniqueOrThrow({ where: { id } })).pinHash!;
    expect(after).not.toBe(before);
    expect(await argon2.verify(after, "987654")).toBe(true);

    expect((await PATCH(req("PATCH", { id, deactivate: true }))).status).toBe(200);
    expect((await prisma.user.findUniqueOrThrow({ where: { id } })).pinHash).toBeNull();

    expect((await PATCH(req("PATCH", { id: foreign.id, deactivate: true }))).status).toBe(404);
    expect((await prisma.user.findUniqueOrThrow({ where: { id: foreign.id } })).pinHash).toBe("h");
    const audits = JSON.stringify(await prisma.auditLog.findMany());
    expect(audits).not.toContain("987654");
    expect(audits).not.toContain(after);
  });
});
