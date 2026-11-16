"""Keep native hardware discovery noise separate from CLI parity failures."""
import importlib.util
from pathlib import Path
import unittest

path = Path(__file__).resolve().parents[1] / "external_tools/markitdown/test_cli.py"
spec = importlib.util.spec_from_file_location("markitdown_cli", path)
cli = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cli)

WARNING = (b'\x1b[0;93m2026-11-16 22:48:16.680522035 '
           b'[W:onnxruntime:Default, device_discovery.cc:95 GetPciBusId] '
           b'Skipping pci_bus_id for PCI path at "/sys/devices/virtual-device" '
           b'because filename "virtual-device" did not match expected pattern of '
           b'[0-9a-f]+:[0-9a-f]+:[0-9a-f]+[.][0-9a-f]+\x1b[m\n')


class NativeDiagnosticsTests(unittest.TestCase):
    def test_known_hardware_warning_is_removed_without_changing_cli_error(self):
        error = b"usage: markitdown\nmarkitdown: error: unrecognized arguments: --unknown\n"
        self.assertEqual(cli.without_onnx_pci_warning(WARNING + error), error)

    def test_other_onnx_warnings_and_cli_errors_are_preserved(self):
        unknown = WARNING.replace(b"GetPciBusId", b"DifferentWarning")
        self.assertEqual(cli.without_onnx_pci_warning(unknown), unknown)
        error = b"markitdown: error: conversion failed\n"
        self.assertEqual(cli.without_onnx_pci_warning(error), error)


if __name__ == "__main__":
    unittest.main()
