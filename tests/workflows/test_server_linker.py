"""Only a known semantic no-op may change on the real cross-link path."""
import unittest

from server_linker import command


class LinkerCompatibilityControls(unittest.TestCase):
    def environment(self, **changes):
        return dict({"CARGO_ZIGBUILD_ZIG_VERSION": "0.16.0",
                     "WORKFLOWS_SERVER_LINKER_TARGET": "aarch64-linux-gnu",
                     "WORKFLOWS_SERVER_LINKER_TOOL": "/tools/cargo-zigbuild"}, **changes)

    def test_should_translate_only_the_exact_obsolete_optimization_argument(self):
        original = ["input with spaces.o", "-Wl,-O1", "-Wl,-O10", "-Wl,-O2", "-O1",
                    "-Wl,-O1,--gc-sections", "-Wl,--version-script=/tmp/list", "-lgcc_s", "-Werror"]
        program, argv = command(original, self.environment())
        self.assertEqual(program, "/tools/cargo-zigbuild")
        self.assertEqual(argv[:8], [program, "zig", "cc", "--", "-g", "-fno-sanitize=all",
                                   "-target", "aarch64-linux-gnu"])
        self.assertEqual(argv[8:], [original[0], *original[2:]])
        self.assertEqual(original[1], "-Wl,-O1")

    def test_should_preserve_all_other_arguments_and_the_selected_target(self):
        for index in range(128):
            original = ["-Wl,--gc-sections", "arg" + str(index), "-Wl,-O" + str(index + 2), "library.rlib"]
            _, argv = command(original, self.environment(WORKFLOWS_SERVER_LINKER_TARGET="x86_64-linux-gnu"))
            self.assertEqual(argv[7], "x86_64-linux-gnu")
            self.assertEqual(argv[8:], original)

    def test_should_reject_unknown_versions_targets_and_tool_paths_before_execution(self):
        for changes in ({"CARGO_ZIGBUILD_ZIG_VERSION": "0.15.2"},
                        {"CARGO_ZIGBUILD_ZIG_VERSION": "0.17.0"},
                        {"WORKFLOWS_SERVER_LINKER_TARGET": "aarch64-linux-musl"},
                        {"WORKFLOWS_SERVER_LINKER_TOOL": "cargo-zigbuild"},
                        {"WORKFLOWS_SERVER_LINKER_TOOL": "/tmp/tool\n"}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                command(["file.o"], self.environment(**changes))

    def test_should_bound_the_complete_argument_vector(self):
        for arguments in ([], ["x"] * 4097, ["x" * 16385], ["x\x00y"], ["x" * 16384] * 257):
            with self.assertRaises(ValueError):
                command(arguments, self.environment())


if __name__ == "__main__":
    unittest.main()
