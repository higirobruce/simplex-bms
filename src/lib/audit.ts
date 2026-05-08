import { prisma } from "@/lib/prisma";
import { Prisma } from "@prisma/client";

interface AuditParams {
  orgId: string;
  userId: string;
  action:
    | "CREATE"
    | "UPDATE"
    | "DELETE"
    | "STATUS_CHANGE"
    | "CONFIRM"
    | "APPROVE"
    | "CANCEL";
  entityType: string;
  entityId: string;
  before?: object | null;
  after?: object | null;
  tx?: Prisma.TransactionClient;
}

export async function logAudit({
  orgId,
  userId,
  action,
  entityType,
  entityId,
  before = null,
  after = null,
  tx,
}: AuditParams) {
  const client = tx || prisma;
  await (client as any).auditLog.create({
    data: {
      orgId,
      userId,
      action,
      entityType,
      entityId,
      before: before ? (before as Prisma.InputJsonValue) : Prisma.JsonNull,
      after: after ? (after as Prisma.InputJsonValue) : Prisma.JsonNull,
    },
  });
}
