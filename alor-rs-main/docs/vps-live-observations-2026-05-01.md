# VPS Live Observations (2026-05-01)

## Checkpoint

Observation time:

```text
2026-05-01 10:22 MSK
host = nektodk.ispvds.com
```

Scope:

```text
sessiongap       USDRUBF / 7502MIW
hybrid           IMOEXF riskgate-shadow / 7502SN6
alor-usdrubf     USDRUBF hybrid / 7502T0U
RI shadow        RIM6 feed -> RI Author41/42 shadow journal
```

## Stack Status

All checked containers were up:

```text
trading-sessiongap-*     up 8 days, healthy
trading-hybrid-*         up 2-3 days, healthy
trading-alor-usdrubf-*   up 4-8 days, healthy
trading-ri-shadow-*      up 2 days
```

VPS resources at checkpoint:

```text
RAM         2.5G used / 7.7G total (5.2G available)
Swap        54M used / 3.9G
Disk /      39G used / 79G (52%)
```

Redis memory:

```text
sessiongap redis      289.58M
hybrid redis          498.71M
alor-usdrubf redis    305.03M
RI shadow redis       329.73M / 512M
```

## Session Regime (Unusual Friday)

Observed bar tails:

```text
hybrid (IMOEXF):      live 10m bars on 2026-05-01
ri-shadow (RIM6):     live 10m bars on 2026-05-01
alor-usdrubf:         latest live activity ended on prior regular session;
                      startup replay contains history_gap bars
sessiongap:           no fresh live trading flow for this holiday-like regime
```

Interpretation: the day behaves like a split regime where IMOEXF/RI continue
trading, while USDRUBF is effectively inactive.

## Log Scan

Checked last ~30h for the three strategy runtimes and RI runner.

Findings:

```text
No panic or fatal runtime errors.
No new command reject storms.
No Redis xreadgroup broken-pipe/connection-refused incident in this window.
```

Seen as expected operational noise:

```text
live_guard_changed transitions during reconnect/sync phases:
  ALLOWED -> BLOCKED -> SyncingGap/SyncingHistory -> ALLOWED
```

This matches previously observed gateway transport behavior and recovered
normally.

## Position Safety

Latest broker position tails indicate flat state for active live stacks:

```text
sessiongap       no open USDRUBF futures position
hybrid           no open IMOEXF futures position
alor-usdrubf     no open USDRUBF futures position
```

RI shadow runner also remained active and continued writing journal updates.

## Verdict

```text
Overall state: штатно.
```

No new blocking incident was found. The main operational watchpoint remains
Redis growth (especially hybrid + RI shadow), but at this checkpoint it is still
within safe operating bounds.

Follow-up:

```text
1. Keep daily Redis memory checks while extended micro soak is active.
2. Keep logging split-regime days explicitly (USDRUBF inactive vs IMOEXF/RI active).
3. If hybrid redis approaches ~600M and growth accelerates, run safe trim in non-trading window.
```
