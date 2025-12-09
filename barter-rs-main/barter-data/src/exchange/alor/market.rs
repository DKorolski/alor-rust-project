use super::Alor;
use crate::{Identifier, instrument::MarketInstrumentData, subscription::Subscription};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub struct AlorMarket {
    pub exchange: SmolStr,
    pub code: SmolStr,
}

impl AlorMarket {
    pub fn new(exchange: impl Into<SmolStr>, code: impl Into<SmolStr>) -> Self {
        Self {
            exchange: exchange.into(),
            code: code.into(),
        }
    }
}

impl<InstrumentKey, Kind> Identifier<AlorMarket>
    for Subscription<Alor, MarketInstrumentData<InstrumentKey>, Kind>
{
    fn id(&self) -> AlorMarket {
        // Use the exchange-specific symbol if provided, otherwise fall back to the external name.
        AlorMarket::new("MOEX", self.instrument.name_exchange.as_ref())
    }
}

impl AsRef<str> for AlorMarket {
    fn as_ref(&self) -> &str {
        &self.code
    }
}