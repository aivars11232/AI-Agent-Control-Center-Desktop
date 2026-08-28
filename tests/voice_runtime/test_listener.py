import contextlib
import importlib.util
import io
import json
import os
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
LISTENER_PATH = REPOSITORY_ROOT / "voice-runtime" / "listener.py"
SPEC = importlib.util.spec_from_file_location("task_0016_listener", LISTENER_PATH)
listener = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(listener)


class ShortReadStream(io.BytesIO):
    def read(self, size=-1):
        return super().read(min(size, 7) if size >= 0 else 7)


class ListenerContractTests(unittest.TestCase):
    def test_short_reads_form_exact_twenty_millisecond_frames(self):
        payload = bytes(range(256)) * 8
        frames = list(
            listener.iter_exact_frames(
                ShortReadStream(payload),
                frame_bytes=listener.FRAME_BYTES,
            )
        )

        self.assertEqual([len(frame) for frame in frames], [640, 640, 640])
        self.assertEqual(b"".join(frames), payload[: 3 * 640])

    def test_preroll_is_copied_once_and_utterances_are_bounded(self):
        capture = listener.UtteranceCapture(preroll_frames=2, maximum_bytes=10)
        self.assertFalse(capture.observe(b"aa", False))
        self.assertTrue(capture.observe(b"bb", True))
        self.assertFalse(capture.observe(b"cc", True))
        self.assertEqual(capture.audio, b"aabbcc")
        capture.observe(b"dddddd", True)
        self.assertTrue(capture.at_limit())
        self.assertEqual(capture.finish(), b"aabbccdddd")
        self.assertFalse(capture.started)
        self.assertEqual(capture.audio, b"")

    def test_invalid_reload_keeps_last_known_good_configuration(self):
        now = [10.0]
        with tempfile.TemporaryDirectory() as directory:
            config_path = Path(directory) / "listener-config.json"
            config_path.write_text(
                json.dumps(
                    {
                        "wakePhrase": "computer",
                        "deactivatePhrase": "computer sleep",
                        "openPhrases": ["open"],
                        "closePhrases": ["close"],
                    }
                ),
                encoding="utf-8",
            )
            reloader = listener.ConfigReloader(
                config_path,
                clock=lambda: now[0],
            )
            original, _changed = reloader.current()
            self.assertEqual(original["wakePhrases"], ["computer"])

            config_path.write_text("{invalid", encoding="utf-8")
            os.utime(config_path, ns=(20_000_000_000, 20_000_000_000))
            now[0] += 1.1
            retained, changed = reloader.current()
            self.assertFalse(changed)
            self.assertEqual(retained, original)

            config_path.write_text(
                json.dumps(
                    {
                        "wakePhrase": "control",
                        "deactivatePhrase": "control sleep",
                        "openPhrases": ["launch"],
                        "closePhrases": ["quit"],
                    }
                ),
                encoding="utf-8",
            )
            os.utime(config_path, ns=(30_000_000_000, 30_000_000_000))
            now[0] += 1.1
            updated, changed = reloader.current()
            self.assertTrue(changed)
            self.assertEqual(updated["wakePhrases"], ["control"])

    @unittest.skipUnless(os.name == "posix", "private executable fixture requires POSIX")
    def test_whisper_temp_audio_is_private_and_removed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "whisper-cli"
            model = root / "model.bin"
            runtime = root / "runtime"
            binary.write_text("#!/bin/sh\nprintf 'local transcript\\n'\n", encoding="utf-8")
            binary.chmod(0o700)
            model.write_bytes(b"model")

            transcript = listener.transcribe_with_whisper(
                str(binary),
                str(model),
                b"\x00\x00" * 320,
                str(runtime),
            )

            self.assertEqual(transcript, "local transcript")
            self.assertEqual(list(runtime.iterdir()), [])
            self.assertEqual(runtime.stat().st_mode & 0o777, 0o700)

    def test_ndjson_output_is_sanitized_and_bounded(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            listener.emit("unknown-kind", "safe\x00text " + "x" * 3_000)
        event = json.loads(output.getvalue())

        self.assertEqual(event["kind"], "error")
        self.assertNotIn("\x00", event["transcript"])
        self.assertLessEqual(
            len(event["transcript"]),
            listener.MAX_TRANSCRIPT_CHARACTERS,
        )


if __name__ == "__main__":
    unittest.main()
