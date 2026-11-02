#!/usr/bin/env python3
"""Exact authored-declaration selection for the unchanged-source export driver."""
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("export_conformance", pathlib.Path(__file__).with_name("export-conformance.py"))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)

class DeclarationSelection(unittest.TestCase):
    def setUp(self):
        self.ledger = {"case": "T00-019", "declarations": [
            {"lean": "T00_019.viaLet"}, {"lean": "T00_019.viaHave"}, {"lean": "T00_019.sharedProof"},
        ]}

    def test_default_exports_every_authored_declaration_in_stable_order(self):
        self.assertEqual(driver.select_declarations(self.ledger, None),
                         ["T00_019.sharedProof", "T00_019.viaHave", "T00_019.viaLet"])

    def test_selection_exports_exact_original_names_without_unrelated_roots(self):
        self.assertEqual(driver.select_declarations(self.ledger, ["T00_019.viaLet", "T00_019.viaHave"]),
                         ["T00_019.viaHave", "T00_019.viaLet"])

    def test_unknown_names_cannot_fabricate_source_evidence(self):
        with self.assertRaisesRegex(ValueError, "not authored"):
            driver.select_declarations(self.ledger, ["T00_019.imaginary"])

    def test_empty_selection_cannot_masquerade_as_a_case_export(self):
        with self.assertRaisesRegex(ValueError, "empty"):
            driver.select_declarations(self.ledger, [])

    def test_repeated_cli_roots_are_exported_once(self):
        self.assertEqual(driver.select_declarations(self.ledger, ["T00_019.viaHave", "T00_019.viaHave"]),
                         ["T00_019.viaHave"])

if __name__ == "__main__":
    unittest.main()
