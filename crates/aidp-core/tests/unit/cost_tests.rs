use super::{parse_prices, PricesOrigin, EMBEDDED_PRICES_JSON};
use crate::models::constants::cost::MODEL_PRICES_ENV;
use std::path::PathBuf;

const INVALID_PRICES_JSON: &str = include_str!("../fixtures/prices/invalid-prices.json");
const INVALID_PRICES_FILE_NAME: &str = "bad-prices.json";

#[test]
fn invalid_override_grid_names_the_env_var_and_absolute_path() {
    let absolute_path: PathBuf = std::env::temp_dir().join(INVALID_PRICES_FILE_NAME);
    let origin = PricesOrigin::Override(absolute_path.clone());

    let Err(error) = parse_prices(INVALID_PRICES_JSON, &origin) else {
        panic!("an invalid grid must be refused");
    };

    assert!(error.contains(&format!("{MODEL_PRICES_ENV}=")), "{error}");
    assert!(
        error.contains(&absolute_path.display().to_string()),
        "{error}"
    );
}

#[test]
fn embedded_grid_parses() {
    let grid = parse_prices(EMBEDDED_PRICES_JSON, &PricesOrigin::Embedded)
        .expect("the embedded grid must parse");

    assert!(!grid.pricing_version.is_empty());
}
