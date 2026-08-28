import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
BASE_SETUP = (REPOSITORY_ROOT / "voice-runtime" / "setup.sh").read_text(
    encoding="utf-8"
)
HIGH_SETUP = (
    REPOSITORY_ROOT / "voice-runtime" / "setup-high-accuracy.sh"
).read_text(encoding="utf-8")


class SetupContractTests(unittest.TestCase):
    def test_base_setup_is_pinned_hashed_staged_and_resumable(self):
        required_contract = [
            'stage_dir="${VOICE_STAGE_DIR',
            'cache_dir="${VOICE_CACHE_DIR',
            "vosk==0.3.45",
            "25e025093c4399d7278f543568ed8cc5460ac3a4bf48c23673ace1e25d26619f",
            "vosk-model-small-en-us-0.15",
            "30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498",
            "--require-hashes",
            "--continue-at -",
            '"kind":"base"',
        ]
        for value in required_contract:
            self.assertIn(value, BASE_SETUP)
        for forbidden in ["torch", "torchaudio", "numpy", "openwakeword", "silero"]:
            self.assertNotIn(forbidden, BASE_SETUP.lower())

    def test_high_accuracy_setup_uses_immutable_source_and_model_hashes(self):
        required_contract = [
            'stage_dir="${VOICE_STAGE_DIR',
            'cache_dir="${VOICE_CACHE_DIR',
            "f049fff95a089aa9969deb009cdd4892b3e74916",
            "279af4ce60dbf397362868f3bacc75b56a4332ac2541cae155070093f6aaf0e3",
            "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
            "147964211",
            "--continue-at -",
            '"kind":"high"',
        ]
        for value in required_contract:
            self.assertIn(value, HIGH_SETUP)
        self.assertNotIn("git clone", HIGH_SETUP)
        self.assertNotIn("--depth", HIGH_SETUP)


if __name__ == "__main__":
    unittest.main()
