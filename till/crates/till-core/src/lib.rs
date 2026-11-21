//! Fiscal core of the Simplex point-of-sale till for Rwandan supermarkets.
//!
//! Currency is RWF and the timezone is Africa/Kigali. Money mirrors the cloud
//! schema's `Decimal(12,2)` and quantity mirrors `Decimal(12,3)`; neither is
//! ever an `f64`.
//!
//! # Why newtypes and not bare `Decimal`
//!
//! The TypeScript half of this project has a documented hazard:
//! `new Prisma.Decimal("10") + 5` silently evaluates to the string `"105"`,
//! which then passed a negative-stock guard and was accepted by the database.
//! Rust removes that class of bug outright, but a money/quantity mix-up is
//! still possible with bare decimals. [`Money`] and [`Qty`] are therefore
//! separate types that cannot be added to one another; the only operation that
//! crosses between them is [`Money::checked_times_qty`].

pub mod codes;
pub mod money;
pub mod tax;

pub use money::{Money, MoneyError, Qty, round_half_up};
