import datetime as dt
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import shadow_closed_bar_source as s


DAY = dt.date(2026, 11, 2)


def bar(ts=None, **kwargs):
    return s.check_bar(dict(time=ts or s.label(DAY, 23, 40), open=100, high=102,
                            low=99, close=101, volume=20, **kwargs))


def config():
    return dict(symbol="IMOEXF", stream="shadow.source.closed10m.IMOEXF.test",
                bootstrap_start="2026-10-28", first_live_session="2026-11-02",
                excluded_dates=["2026-11-04"])


class FinalizerTests(unittest.TestCase):
    def test_terminal_bar_finalizes_without_next_tick(self):
        row = bar()
        f = s.Finalizer(DAY)
        self.assertIsNone(f.observe(row, row["time"] + 605))
        proof = f.observe(row, row["time"] + 610)
        self.assertEqual(proof["origin"], "live")
        self.assertEqual(proof["closed_at_utc"], s.label(DAY, 23, 50))

    def test_current_bar_cannot_finalize(self):
        for age in (0, 599, 600, 604):
            with self.subTest(age=age), self.assertRaises(s.SourceFault):
                s.Finalizer(DAY).observe(bar(), bar()["time"] + age)

    def test_late_bar_is_not_relabelled_history_or_live(self):
        for age in (691, 900, 86400):
            with self.subTest(age=age), self.assertRaises(s.SourceFault):
                s.Finalizer(DAY).observe(bar(), bar()["time"] + age)

    def test_bootstrap_remains_history_with_real_receipt(self):
        row = bar(s.label(DAY - dt.timedelta(days=3), 23, 40))
        now = s.label(DAY - dt.timedelta(days=1), 12)
        f = s.Finalizer(DAY)
        f.observe(row, now)
        proof = f.observe(row, now + 5)
        self.assertEqual(proof["origin"], "history")
        self.assertEqual(proof["confirmed_at_utc"], now + 5)

    def test_changed_candle_requires_new_confirmation(self):
        row = bar()
        changed = dict(row, volume=21.0)
        f = s.Finalizer(DAY)
        f.observe(row, row["time"] + 605)
        self.assertIsNone(f.observe(changed, row["time"] + 610))
        self.assertIsNotNone(f.observe(changed, row["time"] + 615))

    def test_repeated_same_receipt_time_is_not_confirmation(self):
        f = s.Finalizer(DAY)
        f.observe(bar(), bar()["time"] + 605)
        self.assertIsNone(f.observe(bar(), bar()["time"] + 605))

    def test_batch_candidates_do_not_reset_each_other(self):
        a, b = bar(s.label(DAY, 7)), bar(s.label(DAY, 7, 10))
        f = s.Finalizer(DAY + dt.timedelta(days=1))
        now = s.label(DAY, 8)
        f.observe(a, now)
        f.observe(b, now)
        self.assertIsNotNone(f.observe(a, now + 5))
        self.assertIsNotNone(f.observe(b, now + 5))


class ContractTests(unittest.TestCase):
    def test_volume_exception_is_explicit_and_not_available_to_usdrubf(self):
        self.assertEqual(s.revision_policy(config()), s.STRICT_REVISIONS)
        s.validate_config(dict(config(), revision_policy=s.IMOEXF_VOLUME_AUDIT))
        for change in (dict(revision_policy="ignore"), dict(revision_policy=None),
                       dict(symbol="USDRUBF", stream="shadow.source.closed10m.USDRUBF.test",
                            revision_policy=s.IMOEXF_VOLUME_AUDIT)):
            with self.subTest(change=change), self.assertRaises(s.SourceFault):
                s.validate_config(dict(config(), **change))

    def test_next_session_rechecks_previous_terminal_candle(self):
        prior = [s.label(DAY, 23, 30), s.label(DAY, 23, 40)]
        self.assertEqual(s.request_start(s.label(DAY + dt.timedelta(days=1), 7), prior), prior[0])

    def test_market_data_is_always_authenticated_and_has_no_order_endpoint(self):
        with patch.dict(s.os.environ, {"SHADOW_ALOR_REFRESH_TOKEN": "test-only"}):
            api = s.MarketData()
        with patch.object(api, "request", side_effect=[{"AccessToken": "test-access"},
                                                     {"history": [bar()]}]) as call:
            api.bars("IMOEXF", bar()["time"], bar()["time"])
        self.assertTrue(call.call_args_list[0].args[0].startswith("https://oauth.alor.ru/refresh?"))
        self.assertEqual(call.call_args_list[0].kwargs, {"method": "POST"})
        self.assertTrue(call.call_args_list[1].args[0].startswith("https://api.alor.ru/md/v2/history?"))
        self.assertEqual(call.call_args_list[1].args[1], {"Authorization": "Bearer test-access"})
        query = s.urllib.parse.parse_qs(s.urllib.parse.urlparse(call.call_args_list[1].args[0]).query)
        self.assertEqual(int(query["to"][0]), bar()["time"] + 599)

    def test_regular_and_holiday_schedules(self):
        self.assertEqual(len(s.slots(DAY, [])), 101)
        self.assertEqual(len(s.slots(dt.date(2026, 11, 1), [])), 55)
        holiday = dt.date(2026, 11, 4)
        self.assertEqual(len(s.slots(holiday, [holiday.isoformat()])), 55)
        self.assertEqual(list(s.slots(DAY, []))[-1], s.label(DAY, 23, 40))

    def test_context_missing_candle_requires_two_delayed_observations(self):
        ts = s.label(dt.date(2026, 10, 31), 11, 20)
        self.assertIsNone(s.context_gap(ts, ts + 689, ts + 695, []))
        self.assertIsNone(s.context_gap(ts, ts + 690, ts + 694, []))
        proof = s.context_gap(ts, ts + 690, ts + 695, [])
        self.assertEqual(proof["bar_ts_utc"], ts)
        self.assertEqual(s.context_gap(ts, proof["first_absent_at_utc"], proof["confirmed_absent_at_utc"], []), proof)

    def test_regular_missing_candle_never_becomes_context_gap(self):
        ts = s.label(DAY, 23, 40)
        with self.assertRaises(s.SourceFault):
            s.context_gap(ts, ts + 700, ts + 710, [])

    def test_weekend_after_activation_is_context_only_even_if_delayed(self):
        row = bar(s.label(dt.date(2026, 11, 7), 10))
        f = s.Finalizer(DAY)
        f.observe(row, row["time"] + 900)
        self.assertEqual(f.observe(row, row["time"] + 905)["origin"], "history")

    def test_invalid_numbers_and_geometry(self):
        for key, val in [("volume", -1), ("close", float("nan")), ("high", 98),
                         ("open", True), ("low", 0), ("time", bar()["time"] + 1)]:
            with self.subTest(key=key), self.assertRaises(s.SourceFault):
                s.check_bar(dict(bar(), **{key: val}))

    def test_duplicate_api_labels_rejected(self):
        with self.assertRaises(s.SourceFault):
            s.parse_rows({"history": [bar(), bar()]}, 0, bar()["time"])

    def test_api_rounding_outside_request_is_not_published(self):
        row = bar()
        self.assertEqual(s.parse_rows({"history": [row]}, row["time"] + 1, row["time"] + 600), {})

    def test_only_own_stream_and_bounded_seed(self):
        s.validate_config(config())
        for changes in [dict(stream="md.bars.live"), dict(symbol="RIZ6"),
                        dict(bootstrap_start="2026-09-01"), dict(first_live_session="2026-11-01"),
                        dict(unexpected=True)]:
            with self.subTest(changes=changes), self.assertRaises(s.SourceFault):
                s.validate_config(dict(config(), **changes))

    def test_stored_envelope_preserves_receipt_and_provenance(self):
        row = bar()
        f = s.Finalizer(DAY)
        f.observe(row, row["time"] + 605)
        proof = f.observe(row, row["time"] + 610)
        env = s.envelope("IMOEXF", row, proof)
        entry = [str(row["time"] * 1000) + "-0", ["payload", s.canonical(env)]]
        self.assertEqual(s.stored_row(entry, "IMOEXF"), row)
        env["payload"]["c"] += .5
        with self.assertRaises(s.SourceFault):
            s.stored_row([entry[0], ["payload", s.canonical(env)]], "IMOEXF")

    def test_redis_protocol_has_bounded_frames(self):
        for raw in [b"$999999999\r\n", b"*99999999\r\n", b"$2\r\na", b"-ERR secret\r\n"]:
            redis = object.__new__(s.Redis)
            redis.file = io.BytesIO(raw)
            with self.subTest(raw=raw), self.assertRaises(s.SourceFault) as error:
                redis.read()
            self.assertNotIn("secret", str(error.exception))


class RevisionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.row = dict(bar(s.label(dt.date(2026, 11, 1), 17, 40)), volume=589.0)
        self.published = {self.row["time"]: self.row}
        self.now = self.row["time"] + 1206
        self.cfg = dict(config(), revision_policy=s.IMOEXF_VOLUME_AUDIT)

    def tearDown(self):
        self.tmp.cleanup()

    def test_volume_only_revision_preserves_original_and_deduplicates_on_resume(self):
        revised = dict(self.row, volume=590.0)
        with patch.object(s, "log") as log:
            audit = s.RevisionAudit(self.root, self.cfg, self.published)
            audit.check(self.row, revised, self.now)
            before = audit.path.read_bytes()
            audit.check(self.row, revised, self.now + 5)
            resumed = s.RevisionAudit(self.root, self.cfg, self.published)
            resumed.check(self.row, revised, self.now + 10)
            self.assertEqual(before, audit.path.read_bytes())
            self.assertEqual(log.call_count, 1)
        event = json.loads(before)["event"]
        self.assertEqual(event["kind"], "volume_only_revision")
        self.assertEqual(event["action"], "preserve_published")
        self.assertEqual(event["observed"]["volume"], 590.0)
        self.assertEqual(self.published[self.row["time"]]["volume"], 589.0)

    def test_valid_volume_decrease_is_audited_too_not_replaced(self):
        audit = s.RevisionAudit(self.root, self.cfg, self.published)
        with patch.object(s, "log"):
            audit.check(self.row, dict(self.row, volume=588.0), self.now)
        self.assertEqual(self.row["volume"], 589.0)
        self.assertEqual(len(audit.seen), 1)

    def test_default_volume_revision_fails_and_records_evidence(self):
        audit = s.RevisionAudit(self.root, config(), self.published)
        with patch.object(s, "log"), self.assertRaisesRegex(s.SourceFault, "volume_only_revision"):
            audit.check(self.row, dict(self.row, volume=590.0), self.now)
        self.assertEqual(json.loads(audit.path.read_bytes())["event"]["action"], "fail_closed")
        # Even a crash before FAILED.json cannot turn fatal audit into clean resume.
        with self.assertRaisesRegex(s.SourceFault, "requires review"):
            s.RevisionAudit(self.root, config(), self.published)

    def test_price_and_missing_remain_fatal_with_separate_classes(self):
        for revised, reason in ((dict(self.row, close=100.0), "price_revision"),
                                (None, "missing_published_bar")):
            with self.subTest(reason=reason), tempfile.TemporaryDirectory() as directory:
                audit = s.RevisionAudit(Path(directory), self.cfg, self.published)
                with patch.object(s, "log"), self.assertRaisesRegex(s.SourceFault, reason):
                    audit.check(self.row, revised, self.now)
                self.assertEqual(json.loads(audit.path.read_bytes())["event"]["kind"], reason)

    def test_invalid_volume_is_not_an_allowed_revision(self):
        for value in (-1, float("nan"), True):
            audit = s.RevisionAudit(self.root, self.cfg, self.published)
            with self.subTest(value=value), self.assertRaises(s.SourceFault):
                audit.check(self.row, dict(self.row, volume=value), self.now)
            self.assertFalse(audit.path.exists())

    def test_audit_capacity_and_write_failure_do_not_silently_allow_revision(self):
        audit = s.RevisionAudit(self.root, self.cfg, self.published)
        revised = dict(self.row, volume=590.0)
        for limit in ("MAX_REVISION_ROWS", "MAX_REVISION_BYTES"):
            with patch.object(s, limit, 0), self.assertRaisesRegex(s.SourceFault, "capacity"):
                audit.check(self.row, revised, self.now)
        with patch.object(Path, "open", side_effect=OSError("disk full")), self.assertRaises(OSError):
            audit.check(self.row, revised, self.now)
        self.assertFalse(audit.seen)

    def test_torn_or_modified_audit_blocks_resume(self):
        audit = s.RevisionAudit(self.root, self.cfg, self.published)
        with patch.object(s, "log"):
            audit.check(self.row, dict(self.row, volume=590.0), self.now)
        raw = audit.path.read_bytes()
        for broken in (raw[:-1], raw.replace(b'590.0', b'591.0')):
            audit.path.write_bytes(broken)
            with self.assertRaises(s.SourceFault):
                s.RevisionAudit(self.root, self.cfg, self.published)


class SourceIntegrationTests(unittest.TestCase):
    """No network/localhost Redis: drive the real loop with owned fake transports."""
    def test_bootstrap_then_final_terminal_publication_and_resume(self):
        cfg = config()
        terminal = s.label(DAY, 23, 40)
        # Existing stream is the complete deterministic prefix, including today's
        # Live bars. The terminal candle is the only outstanding observation.
        entries = []
        day = dt.date.fromisoformat(cfg["bootstrap_start"])
        while day <= DAY:
            for ts in s.slots(day, cfg["excluded_dates"]):
                if ts >= terminal:
                    break
                row = bar(ts)
                f = s.Finalizer(DAY)
                f.observe(row, ts + 605)
                proof = f.observe(row, ts + 610)
                entries.append(["%d-0" % (ts * 1000), ["payload", s.canonical(s.envelope("IMOEXF", row, proof))]])
            day += dt.timedelta(days=1)
        fake_time = [terminal + 605]
        handlers = {}

        class FakeRedis:
            def close(self):
                pass

            def command(self, command, *args):
                if command == "XLEN":
                    return len(entries)
                if command == "XRANGE":
                    return entries[:]
                if command == "XADD":
                    entries.append([args[1], [args[2], args[3]]])
                    handlers[s.signal.SIGTERM](None, None)
                    return args[1]
                raise AssertionError(command)

        class FakeApi:
            def bars(self, symbol, start, end):
                return {ts: bar(ts) for ts in range(start, end + 1, 600)}

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "manifest.json").write_text(s.canonical(dict(version=s.VERSION, config=cfg)))
            with patch.object(s, "Redis", FakeRedis), patch.object(s, "MarketData", FakeApi), \
                    patch.object(s.signal, "signal", lambda sig, handler: handlers.update({sig: handler})), \
                    patch.object(s.time, "time", lambda: fake_time[0]), \
                    patch.object(s.time, "monotonic", lambda: fake_time[0]), \
                    patch.object(s.time, "sleep", lambda n: fake_time.__setitem__(0, fake_time[0] + n)), \
                    patch.object(s, "log"):
                s.run(cfg, root)
                self.assertFalse((root / "FAILED.json").exists())
                self.assertEqual(s.stored_row(entries[-1], "IMOEXF")["time"], terminal)
                initial_count = len(entries)
                # A clean restart reads the canonical stream, emits no duplicate.
                def stop_sleep(_n):
                    handlers[s.signal.SIGTERM](None, None)
                with patch.object(s.time, "sleep", stop_sleep):
                    s.run(cfg, root)
                self.assertEqual(len(entries), initial_count)

    def test_empty_existing_generation_is_not_silently_recreated(self):
        cfg = config()
        class EmptyRedis:
            def close(self):
                pass

            def command(self, *args):
                return 0
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "manifest.json").write_text(s.canonical(dict(version=s.VERSION, config=cfg)))
            with patch.object(s, "Redis", EmptyRedis), patch.object(s.signal, "signal"), \
                    self.assertRaisesRegex(s.SourceFault, "missing/empty"):
                s.run(cfg, root)

    def test_revision_policy_runs_through_real_loop_without_rewriting_source(self):
        for policy, change, failure in (
                (s.IMOEXF_VOLUME_AUDIT, dict(volume=21.0), None),
                (s.STRICT_REVISIONS, dict(volume=21.0), "volume_only_revision"),
                (s.IMOEXF_VOLUME_AUDIT, dict(close=100.0), "price_revision"),
                (s.IMOEXF_VOLUME_AUDIT, None, "missing_published_bar")):
            with self.subTest(policy=policy, change=change), tempfile.TemporaryDirectory() as directory:
                cfg = dict(config(), revision_policy=policy)
                root = Path(directory)
                terminal = s.label(DAY, 23, 40)
                entries = []
                day = dt.date.fromisoformat(cfg["bootstrap_start"])
                while day <= DAY:
                    for ts in s.slots(day, cfg["excluded_dates"]):
                        if ts >= terminal:
                            break
                        row = bar(ts)
                        finalizer = s.Finalizer(DAY)
                        finalizer.observe(row, ts + 605)
                        proof = finalizer.observe(row, ts + 610)
                        entries.append([f"{ts * 1000}-0", ["payload", s.canonical(s.envelope("IMOEXF", row, proof))]])
                    day += dt.timedelta(days=1)
                before = json.dumps(entries)
                count = len(entries)
                handlers, clock = {}, [terminal + 605]
                class FakeRedis:
                    def close(self):
                        pass
                    def command(self, cmd, *args):
                        if cmd == "XLEN":
                            return len(entries)
                        if cmd == "XRANGE":
                            return entries[:]
                        if cmd == "XADD":
                            entries.append([args[1], [args[2], args[3]]])
                            handlers[s.signal.SIGTERM](None, None)
                            return args[1]
                        raise AssertionError(cmd)
                class FakeApi:
                    def bars(self, symbol, start, end):
                        rows = {ts: bar(ts) for ts in range(start, end + 1, 600)}
                        if change is None:
                            del rows[terminal - 600]
                        else:
                            rows[terminal - 600].update(change)
                        return rows
                (root / "manifest.json").write_text(s.canonical(dict(version=s.VERSION, config=cfg)))
                with patch.object(s, "Redis", FakeRedis), patch.object(s, "MarketData", FakeApi), \
                        patch.object(s.signal, "signal", lambda sig, f: handlers.update({sig: f})), \
                        patch.object(s.time, "time", lambda: clock[0]), \
                        patch.object(s.time, "monotonic", lambda: clock[0]), \
                        patch.object(s.time, "sleep", lambda n: clock.__setitem__(0, clock[0] + n)), \
                        patch.object(s, "log"):
                    if failure:
                        with self.assertRaisesRegex(s.SourceFault, failure):
                            s.run(cfg, root)
                        self.assertEqual(json.loads((root / "FAILED.json").read_text())["reason"], failure)
                        self.assertEqual(len(entries), count)
                        with self.assertRaisesRegex(s.SourceFault, "failed source generation"):
                            s.run(cfg, root)
                    else:
                        s.run(cfg, root)
                        self.assertEqual(len(entries), count + 1)
                        self.assertEqual(s.stored_row(entries[-1], "IMOEXF")["time"], terminal)
                        self.assertFalse((root / "FAILED.json").exists())
                self.assertEqual(json.dumps(entries[:count]), before)
                self.assertEqual(len((root / "revisions.jsonl").read_text().splitlines()), 1)

    def test_v2_generation_cannot_be_resumed_by_new_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "manifest.json").write_text(s.canonical(dict(
                version="shadow_authenticated_rest_closed_m10_v2", config=config())))
            with self.assertRaisesRegex(s.SourceFault, "manifest changed"):
                s.run(config(), root)


if __name__ == "__main__":
    unittest.main()
