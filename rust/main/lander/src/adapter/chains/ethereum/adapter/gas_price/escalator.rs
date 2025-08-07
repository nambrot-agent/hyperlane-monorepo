use ethers::types::transaction::eip2718::TypedTransaction::{Eip1559, Eip2930, Legacy};
use ethers_core::types::transaction::eip2718::TypedTransaction;
use tracing::{debug, error, info, warn};

use hyperlane_core::U256;
use hyperlane_ethereum::TransactionOverrides;

use crate::adapter::EthereumTxPrecursor;

use super::price::GasPrice;

/// Configuration for the gas price escalator.
#[derive(Clone, Copy, Debug)]
pub struct GasEscalatorConfig {
    pub escalation_multiplier_numerator: u32,
    pub escalation_multiplier_denominator: u32,
    pub twap_history_weight: u32,
    pub twap_current_weight: u32,
}

impl Default for GasEscalatorConfig {
    fn default() -> Self {
        Self {
            escalation_multiplier_numerator: 110,
            escalation_multiplier_denominator: 100,
            twap_history_weight: 3,
            twap_current_weight: 1,
        }
    }
}

impl From<&TransactionOverrides> for GasEscalatorConfig {
    fn from(overrides: &TransactionOverrides) -> Self {
        let default = GasEscalatorConfig::default();
        Self {
            escalation_multiplier_numerator: overrides
                .gas_escalator_multiplier_numerator
                .map(|v| v.as_u32())
                .unwrap_or(default.escalation_multiplier_numerator),
            escalation_multiplier_denominator: overrides
                .gas_escalator_multiplier_denominator
                .map(|v| v.as_u32())
                .unwrap_or(default.escalation_multiplier_denominator),
            twap_history_weight: overrides
                .gas_escalator_history_weight
                .map(|v| v.as_u32())
                .unwrap_or(default.twap_history_weight),
            twap_current_weight: overrides
                .gas_escalator_current_weight
                .map(|v| v.as_u32())
                .unwrap_or(default.twap_current_weight),
        }
    }
}

/// Sets the max between the newly estimated gas price and 1.1x the old gas price.
pub fn escalate_gas_price_if_needed(
    old_gas_price: &GasPrice,
    estimated_gas_price: &GasPrice,
    config: &GasEscalatorConfig,
) -> GasPrice {
    // assumes the old and new txs have the same type
    match (old_gas_price, estimated_gas_price) {
        (GasPrice::None, _) => {
            // If the old gas price is None, we do not escalate.
            info!(
                ?old_gas_price,
                ?estimated_gas_price,
                "No gas price set on old transaction precursor, skipping escalation"
            );
            GasPrice::None
        }
        (_, GasPrice::None) => {
            // If the estimated gas price is None, we do not escalate.
            info!(
                ?old_gas_price,
                ?estimated_gas_price,
                "Estimated gas price is None, skipping escalation"
            );
            GasPrice::None
        }
        (
            GasPrice::NonEip1559 {
                gas_price: old_gas_price,
            },
            GasPrice::NonEip1559 {
                gas_price: estimated_gas_price,
            },
        ) => {
            let escalated_gas_price =
                get_escalated_price_from_old_and_new(old_gas_price, estimated_gas_price, config);
            debug!(
                tx_type = "Legacy or Eip2930",
                ?old_gas_price,
                ?escalated_gas_price,
                "Escalation attempt outcome"
            );

            GasPrice::NonEip1559 {
                gas_price: escalated_gas_price,
            }
        }
        (
            GasPrice::Eip1559 {
                max_fee: old_max_fee,
                max_priority_fee: old_max_priority_fee,
            },
            GasPrice::Eip1559 {
                max_fee: new_max_fee,
                max_priority_fee: new_max_priority_fee,
            },
        ) => {
            let escalated_max_fee_per_gas =
                get_escalated_price_from_old_and_new(old_max_fee, new_max_fee, config);

            let escalated_max_priority_fee_per_gas = get_escalated_price_from_old_and_new(
                old_max_priority_fee,
                new_max_priority_fee,
                config,
            );

            debug!(
                tx_type = "Eip1559",
                old_max_fee_per_gas = ?old_max_fee,
                escalated_max_fee_per_gas = ?escalated_max_fee_per_gas,
                old_max_priority_fee_per_gas = ?old_max_priority_fee,
                escalated_max_priority_fee_per_gas = ?escalated_max_priority_fee_per_gas,
                "Escalation attempt outcome"
            );

            GasPrice::Eip1559 {
                max_fee: escalated_max_fee_per_gas,
                max_priority_fee: escalated_max_priority_fee_per_gas,
            }
        }
        (old, new) => {
            error!(?old, ?new, "Newly estimated transaction type does not match the old transaction type. Not escalating gas price.");
            GasPrice::None
        }
    }
}

fn get_escalated_price_from_old_and_new(
    old_gas_price: &U256,
    new_gas_price: &U256,
    config: &GasEscalatorConfig,
) -> U256 {
    // Blend the previous gas price with the most recently estimated gas price to
    // form a simple TWAP. This smooths out short lived spikes in the oracle and
    // prevents the escalator from growing without bound when prices are
    // volatile.
    let history_weight = U256::from(config.twap_history_weight);
    let current_weight = U256::from(config.twap_current_weight);
    let blended = (*old_gas_price * history_weight + *new_gas_price * current_weight)
        / (history_weight + current_weight);

    // Apply the escalation multiplier to the blended price and the oracle price
    // to protect against stale oracle data.
    let escalated_from_history = apply_escalation_multiplier(&blended, config);
    let escalated_from_oracle = apply_escalation_multiplier(new_gas_price, config);
    escalated_from_history.max(escalated_from_oracle)
}

fn apply_escalation_multiplier(gas_price: &U256, config: &GasEscalatorConfig) -> U256 {
    let numerator = U256::from(config.escalation_multiplier_numerator);
    let denominator = U256::from(config.escalation_multiplier_denominator);
    gas_price * numerator / denominator
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twap_escalates_beyond_new_price() {
        let old = U256::from(200_000u64);
        let oracle = U256::from(219_000u64);
        let config = GasEscalatorConfig::default();
        let escalated = get_escalated_price_from_old_and_new(&old, &oracle, &config);

        // With the TWAP weighting the escalation still ensures we pay above the
        // latest oracle price.
        assert_eq!(escalated, U256::from(240_900u64));
    }

    #[test]
    fn config_overrides_defaults() {
        let overrides = TransactionOverrides {
            gas_escalator_multiplier_numerator: Some(200u32.into()),
            gas_escalator_multiplier_denominator: Some(100u32.into()),
            gas_escalator_history_weight: Some(1u32.into()),
            gas_escalator_current_weight: Some(1u32.into()),
            ..Default::default()
        };

        let config = GasEscalatorConfig::from(&overrides);
        assert_eq!(config.escalation_multiplier_numerator, 200);
        assert_eq!(config.twap_history_weight, 1);
        assert_eq!(config.twap_current_weight, 1);
    }
}
