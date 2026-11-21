import ast
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import export_component_reaction_reference as exporter


class ComponentReferenceExportTests(unittest.TestCase):
    def model(self, root, name, content):
        path = root / name / 'analysis/live_baseline_revalidation/imoexf_author_comparison/model.py'
        path.parent.mkdir(parents=True)
        path.write_bytes(content)
        return path

    def test_external_model_is_selected_by_content_not_private_directory_name(self):
        content = b'def replay(): pass\n'
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(
                exporter, 'MODEL_SHA', hashlib.sha256(content).hexdigest()):
            root = Path(temp)
            self.model(root, 'unrelated-vintage', b'wrong model')
            expected = self.model(root, 'external-model-fixture', content)
            self.assertEqual(exporter.resolve_model(root), expected)

    def test_missing_changed_or_ambiguous_external_model_is_rejected(self):
        content = b'def replay(): pass\n'
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(
                exporter, 'MODEL_SHA', hashlib.sha256(content).hexdigest()):
            root = Path(temp)
            for step in range(3):
                if step == 1:
                    self.model(root, 'changed-vintage', b'wrong model')
                elif step == 2:
                    self.model(root, 'first-vintage', content)
                    self.model(root, 'second-vintage', content)
                with self.subTest(step=step), self.assertRaisesRegex(ValueError, 'exactly one pinned'):
                    exporter.resolve_model(root)

    def test_function_slice_never_executes_module_side_effects(self):
        source = "raise RuntimeError('module side effect')\ndef selected():\n    return 42\n"
        env = {}
        exec(compile(exporter.function_tree(source, ("selected",)), "fixture", "exec"), env)
        self.assertEqual(env["selected"](), 42)

    def test_missing_or_duplicate_reference_function_is_rejected(self):
        for source in ("def other(): pass", "def selected(): pass\ndef selected(): pass"):
            with self.subTest(source=source):
                with self.assertRaisesRegex(ValueError, "function inventory changed"):
                    exporter.function_tree(source, ("selected",))

    def test_unknown_control_site_cannot_silently_gain_a_trace_label(self):
        tree = ast.parse("for i in range(1):\n    if new_branch:\n        continue")
        with self.assertRaisesRegex(ValueError, "unclassified original continue"):
            exporter.TagContinues().visit(tree)

    def test_trace_tag_preserves_continue_and_classifies_original_ownership_guard(self):
        tree = ast.parse("for i in range(1):\n    if position is not None or pending is not None:\n        continue")
        tree = ast.fix_missing_locations(exporter.TagContinues().visit(tree))
        env = {"position": object(), "pending": None}
        exec(compile(tree, "fixture", "exec"), env)
        self.assertEqual(env["__fixture_site"], "busy_or_invalid")
        self.assertEqual(sum(isinstance(n, ast.Continue) for n in ast.walk(tree)), 1)


if __name__ == "__main__":
    unittest.main()
