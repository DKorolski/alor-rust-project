# Notes

## Bar timestamp semantics

`close_time_utc` в текущем pipeline трактуется как `bar_start_ts_utc` (левая граница интервала).

Это соглашение должно оставаться единообразным в:
- gateway JSON событиях,
- CSV dump/replay,
- runtime и стратегиях.
