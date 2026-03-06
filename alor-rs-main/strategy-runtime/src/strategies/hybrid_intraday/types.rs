use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    MeanReversion,
    IntradayBreakout,
}

impl Owner {
    pub fn as_str(self) -> &'static str {
        match self {
            Owner::MeanReversion => "mean_reversion",
            Owner::IntradayBreakout => "intraday_breakout",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Long,
    Short,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Long => "long",
            Side::Short => "short",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryStyle {
    Bracket,
    Market,
}

impl EntryStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            EntryStyle::Bracket => "bracket",
            EntryStyle::Market => "market",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    MorningMeanReversionLong,
    MorningMeanReversionShort,
    BreakoutLong,
    BreakoutShort,
    BreakoutEodExit,
    BreakoutStop2Long,
    BreakoutStop1Long,
    BreakoutStop2Short,
    BreakoutStop1Short,
    MeanRevTimeCutoff,
    WaitfixOvernightExit,
}

impl ReasonCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ReasonCode::MorningMeanReversionLong => "morning_mean_reversion_long",
            ReasonCode::MorningMeanReversionShort => "morning_mean_reversion_short",
            ReasonCode::BreakoutLong => "breakout_long",
            ReasonCode::BreakoutShort => "breakout_short",
            ReasonCode::BreakoutEodExit => "breakout_eod_exit",
            ReasonCode::BreakoutStop2Long => "breakout_stop2_long",
            ReasonCode::BreakoutStop1Long => "breakout_stop1_long",
            ReasonCode::BreakoutStop2Short => "breakout_stop2_short",
            ReasonCode::BreakoutStop1Short => "breakout_stop1_short",
            ReasonCode::MeanRevTimeCutoff => "mean_rev_time_cutoff",
            ReasonCode::WaitfixOvernightExit => "waitfix_overnight_exit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntrySignal {
    pub owner: Owner,
    pub side: Side,
    pub entry_style: EntryStyle,
    pub reason: ReasonCode,
    pub stop_price: Option<f64>,
    pub take_price: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ExitSignal {
    pub owner: Owner,
    pub reason: ReasonCode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Action {
    SubmitEntry(EntrySignal),
    SubmitExit {
        owner: Owner,
        reason: ReasonCode,
    },
    ArmOvernightExit {
        owner: Owner,
        reason: ReasonCode,
        armed_date: NaiveDate,
        exit_time: NaiveTime,
    },
}
