import math
import unittest

import export_component_reaction_history as exporter


class Row(dict):
    __getattr__ = dict.__getitem__


class ComponentHistoryExportTests(unittest.TestCase):
    def row(self):
        return Row(prev_close=100., prev_high=102., prev_low=98., prev_range=4.,
                   prev2_close=99., prev_return=100./99.-1)

    def test_independent_entry_and_stop_scales_remain_full_previous_range(self):
        values = exporter.component_features(self.row())
        self.assertEqual(values[-2:], [4., 4.])
        self.assertEqual(len(values), 8)

    def test_unknown_prior_close_never_becomes_zero_or_an_older_anchor(self):
        row = self.row()
        row["prev2_close"] = math.nan
        self.assertIsNone(exporter.component_features(row))

    def test_flat_but_known_session_stays_data_ready(self):
        row = self.row()
        row.update(prev_high=100., prev_low=100., prev_range=0.)
        self.assertEqual(exporter.component_features(row)[-2:], [0., 0.])

    def test_primary_controls_do_not_add_an_unapproved_model_or_usd_arm(self):
        self.assertEqual(exporter.ARMS, (
            ("IMO_AA", "IMO_AA", "mr_A_bo_A"),
            ("IMO_AB", "IMO_AB", "mr_A_bo_B"),
            ("IMO_AB_OI_OFF", "IMO_AB_OI", "mr_A_bo_B"),
        ))


if __name__ == "__main__":
    unittest.main()
