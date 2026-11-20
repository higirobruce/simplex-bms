/** Single source of truth for "is this the test database". Throws unless it is bms_test. */
export function assertTestDatabaseUrl(url: string | undefined): void {
  if (!/\/bms_test(\?|$)/.test(url ?? "")) {
    throw new Error(
      "Refusing to run database tests: DATABASE_URL does not point at bms_test. Is .env.test present?",
    );
  }
}
