//! `--demo`: a small catalogue so the lane is usable from the keyboard alone.
//! These are the products in the approved mockup, with its barcodes and prices.

use crate::lane::LaneConfig;
use till_core::Money;
use till_core::codes::TaxType;
use till_core::scan::{EmbeddedMode, ean13_check_digit};
use till_store::{CachedProduct, Counter};

/// A placeholder item class: the lane refuses a product without one, so the
/// demo gives every product one. It is NOT a real RRA classification for these
/// goods; the back office owns that list.
const DEMO_ITEM_CLS: &str = "5020230602";

/// PLU of the weighed product (tomatoes).
pub const TOMATO_PLU: &str = "4087";

fn product(
    id: &str,
    name: &str,
    barcode: Option<&str>,
    plu: Option<&str>,
    price: &str,
    band: TaxType,
    weighed: bool,
) -> CachedProduct {
    CachedProduct {
        id: id.into(),
        sku: plu.unwrap_or(id).into(),
        name: name.into(),
        barcode: barcode.map(Into::into),
        plu: plu.map(Into::into),
        unit_price: Money::try_from_str(price).expect("demo price is valid"),
        tax_ty_cd: band,
        is_weighed: weighed,
        item_cls_cd: Some(DEMO_ITEM_CLS.into()),
    }
}

pub fn products() -> Vec<CachedProduct> {
    vec![
        product(
            "milk",
            "Inyange Fresh Milk 1L",
            Some("5612345000017"),
            Some("6001"),
            "1200",
            TaxType::B,
            false,
        ),
        product(
            "sugar",
            "Kabuye Sugar 1kg",
            Some("5612345000024"),
            Some("6014"),
            "1500",
            TaxType::B,
            false,
        ),
        product(
            "tomato",
            "Tomatoes",
            None,
            Some(TOMATO_PLU),
            "800",
            TaxType::A,
            true,
        ),
        product(
            "cola",
            "Coca-Cola 500ml",
            Some("5449000000996"),
            None,
            "500",
            TaxType::B,
            false,
        ),
        product(
            "mutzig",
            "Mutzig 650ml",
            Some("5612345000031"),
            None,
            "1200",
            TaxType::B,
            false,
        ),
    ]
}

/// The scale label a till would scan for `grams` of tomatoes (weight mode:
/// prefix `20`, five-digit item ref, five digits of grams, check digit).
pub fn tomato_label(grams: u32) -> String {
    let first12 = format!("20{TOMATO_PLU:0>5}{grams:05}");
    let check = ean13_check_digit(&first12).expect("12 digits");
    format!("{first12}{check}")
}

pub fn config() -> LaneConfig {
    LaneConfig {
        dvc_srl_no: "1234567890".into(),
        bhf_id: "00".into(),
        embedded: EmbeddedMode::Weight,
        counter: Counter::SaleInvc,
        shop_name: "LA GARDIENNE".into(),
        shop_address: "Kigali · Nyarugenge".into(),
        shop_tin: "102938475".into(),
    }
}

/// What to tell the person at the keyboard.
pub fn cheat_sheet() -> String {
    format!(
        "Demo catalogue. Type a barcode or PLU and press Enter:\n  \
         5612345000017  Inyange Fresh Milk 1L (PLU 6001)   1,200 B\n  \
         5612345000024  Kabuye Sugar 1kg (PLU 6014)         1,500 B\n  \
         {}  Tomatoes 1.250 kg (scale label)      800/kg A\n  \
         5449000000996  Coca-Cola 500ml                      500 B\n  \
         5612345000031  Mutzig 650ml                       1,200 B\n\
         Then F8 (cash; type an amount first, or none for exact) and Enter to pay.",
        tomato_label(1250)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lane::LaneState;
    use till_core::scan::{Scanned, classify};
    use till_store::Store;

    #[test]
    fn every_demo_barcode_passes_its_check_digit() {
        for p in products() {
            if let Some(b) = &p.barcode {
                assert!(
                    matches!(classify(b, EmbeddedMode::Weight), Ok(Scanned::Barcode(_))),
                    "{b}"
                );
            }
        }
    }

    #[test]
    fn the_tomato_label_decodes_to_1_250_kg_of_the_tomato_plu() {
        match classify(&tomato_label(1250), EmbeddedMode::Weight).unwrap() {
            Scanned::WeightEmbedded { item_ref, qty, .. } => {
                assert_eq!(item_ref, TOMATO_PLU);
                assert_eq!(qty.unwrap().to_string(), "1.250");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_demo_product_is_sellable_through_the_lane() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let store = Store::open_in_memory().await.unwrap();
            store.sync_catalogue(&products()).await.unwrap();
            store
                .open_shift(&config().dvc_srl_no, "cashier", Money::zero())
                .await
                .unwrap();
            let mut lane = LaneState::new(store, config()).await.unwrap();
            let mut codes: Vec<String> = products()
                .iter()
                .filter_map(|p| p.barcode.clone())
                .collect();
            codes.push(tomato_label(1250));
            for code in &codes {
                code.chars().for_each(|c| lane.key(c));
                lane.enter().await.unwrap();
            }
            assert_eq!(lane.basket().len(), codes.len());
        });
    }
}
