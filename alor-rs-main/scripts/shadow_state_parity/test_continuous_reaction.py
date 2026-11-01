import copy
import os
from pathlib import Path
import tempfile
import unittest

from compare import ParityError
from continuous_reaction import fixture, expected, verify_trace, check_delta, digest_id, identities, EVIDENCE, check_fixture_status_shape
from export_imoexf import reference_modules
from reaction_fixtures import configs
from reaction_history import PriceContext
from test_reaction_history import prices


class ContinuousFixtureTests(unittest.TestCase):
    def setUp(self):
        self.config = configs()[2]
        self.prices = PriceContext(prices(),7)
        self.f = fixture(self.config,self.prices,"2026-10-26","2026-10-26")

    def test_declared_clock_complete_grid_restart_and_input_only(self):
        self.assertEqual(self.f["evidence_kind"],EVIDENCE)
        self.assertEqual(sum(o["kind"]=="restart" for o in self.f["operations"]),3)
        self.assertNotIn("expected",self.f)
        times = [o["now"] for o in self.f["operations"]]
        self.assertEqual(times,sorted(times))
        for o in self.f["operations"]:
            if o["kind"]=="receipt":
                self.assertEqual(o["bar"]["close_time_utc"]+600,o["now"])
                self.assertEqual(o["bar"]["origin"],"live")

    def test_missing_execution_day_or_late_eod_cannot_be_omitted(self):
        incomplete = PriceContext(prices().iloc[:-1],7)
        for start,end,p in (("2026-10-26","2026-10-27",self.prices),("2026-10-26","2026-10-26",incomplete),("2026-10-25","2026-10-26",self.prices)):
            with self.assertRaises(ParityError): fixture(self.config,p,start,end)

    def test_unapproved_profiles_rejected(self):
        for cfg in configs()[-2:]:
            with self.assertRaises(ParityError): fixture(cfg,self.prices,"2026-10-26","2026-10-26")

    def test_bootstrap_diagnostics_do_not_relabel_original_receipt_evidence(self):
        status = dict.fromkeys("committed_transactions durable_receipts ingested_sequence observed_sequence transaction_bytes source_caught_up failed session_date pair_ready session_complete order_emission_allowed".split())
        check_fixture_status_shape(status)
        check_fixture_status_shape(dict(status, bootstrap_status="disabled", bootstrap_rows=0))
        for extra in (dict(bootstrap_status="loaded", bootstrap_rows=202),
                      dict(bootstrap_status="disabled"), dict(bootstrap_rows=0),
                      dict(bootstrap_status="disabled", bootstrap_rows=1)):
            with self.assertRaises(ParityError): check_fixture_status_shape(dict(status, **extra))
        candidate = copy.deepcopy(self.f)
        candidate["spec"]["bootstrap"] = {}
        with self.assertRaises(ParityError): verify_trace(candidate, [], Path("not-read.jsonl"))

    def test_delta_ids_are_policy_scoped_and_duplicates_rejected(self):
        ids = identities(self.f); generation = self.f["spec"]["ingress"]["generation"]
        payload = dict(role="entry",component="MR",side="short",ts="2026-10-26T07:10:00",signal_ts="2026-10-26T07:00:00",price=100.1)
        ref = dict(role="entry",owner="MR",side="short",action=payload["ts"],signal=payload["signal_ts"],price=100.1,reason=None)
        want = dict(events=[(0,ref)],trades=[])
        record = dict(id=digest_id(generation,ids[0],"2026-10-26","events",0),payload=payload)
        actual = dict(events=[record],trades=[]); seen=set()
        check_delta(actual,want,ids[0],generation,"2026-10-26",seen)
        for identity in ids:
            with self.assertRaises(ParityError): check_delta(actual,want,identity,generation,"2026-10-26",seen)
        bad=copy.deepcopy(actual);bad["events"][0]["payload"]["price"]+=1
        with self.assertRaises(ParityError): check_delta(bad,want,ids[0],generation,"2026-10-26",set())

    @unittest.skipUnless(os.environ.get("IMOEXF_STATE_HANDOFF"),"explicit immutable handoff required")
    def test_oracle_eod_and_truncated_actual_fail_closed(self):
        with reference_modules(Path(os.environ["IMOEXF_STATE_HANDOFF"])) as modules:
            rows=list(expected(self.f,self.prices,modules))
        model=[r for r in rows if r.get("models") is not None and r["kind"]=="receipt"]
        self.assertEqual(len(model),101)
        self.assertTrue(all(m["pending"] is None and m["position"] is None for m in model[-1]["models"]))
        self.assertTrue(any(r["deltas"][0]["trades"] for r in model))
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/"empty.jsonl";path.touch()
            with self.assertRaises(ParityError): verify_trace(self.f,rows,path)


if __name__ == "__main__": unittest.main()
