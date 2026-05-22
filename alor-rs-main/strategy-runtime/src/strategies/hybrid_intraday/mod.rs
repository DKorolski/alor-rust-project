pub mod high180;
pub mod intraday_breakout;
pub mod mean_reversion;
pub mod orchestrator;
pub mod risk_gate;
pub mod types;

pub use high180::{High180MrConfig, High180MrEngine, High180Open, High180Signal};
pub use intraday_breakout::{IntradayBreakoutConfig, IntradayBreakoutEngine, MinRangeMode};
pub use mean_reversion::{MeanReversionConfig, MeanReversionEngine};
pub use orchestrator::{
    BreakoutEodMode, HybridOrchestrator, HybridOrchestratorConfig, HybridState,
};
pub use risk_gate::{
    build_ledger_records_from_rows, build_runtime_session_row, mr_enabled_for_next_session,
    mr_enabled_for_session, parse_seed_csv, plan_risk_gate_startup,
    rebuild_materialized_state_from_ledger_records, reconcile_seed_with_ledger,
    rolling_shadow_pnl_before_next_session, rows_from_ledger_records,
    validate_ledger_record_identity, validate_regular_session_ledger, RiskGateLedgerRecord,
    RiskGateMaterializedState, RiskGateProfileIdentity, RiskGateRedisKeys, RiskGateRowSource,
    RiskGateRowStatus, RiskGateSessionRow, RiskGateStartupArtifacts, RiskGateStartupDecision,
    RiskGateStartupMode, RISK_GATE_STATE_GENERATION, SHADOW_PNL_LB120_LOOKBACK_SESSIONS,
    SHADOW_PNL_LB120_MIN_HISTORY_SESSIONS,
};
pub use types::{Action, EntrySignal, EntryStyle, ExitSignal, Owner, ReasonCode, Side};
