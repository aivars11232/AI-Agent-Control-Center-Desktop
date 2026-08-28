import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import wave
from collections import deque

SAMPLE_RATE = 16_000
SAMPLE_WIDTH = 2
FRAME_MILLISECONDS = 20
FRAME_BYTES = SAMPLE_RATE * SAMPLE_WIDTH * FRAME_MILLISECONDS // 1_000
PREROLL_FRAMES = 15
MAX_UTTERANCE_BYTES = SAMPLE_RATE * SAMPLE_WIDTH * 20
MAX_TRANSCRIPT_CHARACTERS = 2_048
MAX_PROCESS_OUTPUT_BYTES = 64 * 1_024
CONFIG_POLL_SECONDS = 1.0

VOICE_OFF_PHRASES = {
    "lucy turn voice control off",
    "lucy disable voice control",
}

DEFAULT_CONFIG = {
    "wakePhrase": "lucy",
    "deactivatePhrase": "lucy deactivate",
    "openPhrases": ["open", "launch", "start"],
    "closePhrases": ["close", "quit", "exit"],
}

ALLOWED_EVENT_KINDS = {
    "activated",
    "command",
    "deactivated",
    "error",
    "heard",
    "listening",
    "off_requested",
    "ready",
    "transcribing",
    "warning",
}


def normalize_phrase(value, fallback, maximum=80):
    phrase = value.strip().lower() if isinstance(value, str) else ""
    if not phrase or len(phrase) > maximum:
        return fallback
    return " ".join(phrase.split())


def phrase_variants(value, fallback):
    phrases = [
        normalize_phrase(phrase, "", 80)
        for phrase in normalize_phrase(value, fallback, 160).split(",")
    ]
    result = [phrase for phrase in phrases if phrase][:12]
    return result or [fallback]


def normalize_phrase_list(value, fallback):
    if not isinstance(value, list):
        return list(fallback)
    phrases = [normalize_phrase(item, "", 40) for item in value]
    result = [phrase for phrase in phrases if phrase][:12]
    return result or list(fallback)


def parse_config(saved):
    if not isinstance(saved, dict):
        raise ValueError("configuration must be an object")
    return {
        "wakePhrases": phrase_variants(
            saved.get("wakePhrase"), DEFAULT_CONFIG["wakePhrase"]
        ),
        "deactivatePhrases": phrase_variants(
            saved.get("deactivatePhrase"), DEFAULT_CONFIG["deactivatePhrase"]
        ),
        "openPhrases": normalize_phrase_list(
            saved.get("openPhrases"), DEFAULT_CONFIG["openPhrases"]
        ),
        "closePhrases": normalize_phrase_list(
            saved.get("closePhrases"), DEFAULT_CONFIG["closePhrases"]
        ),
    }


def read_config(config_path):
    with open(config_path, encoding="utf-8") as config_file:
        return parse_config(json.load(config_file))


class ConfigReloader:
    def __init__(self, config_path, clock=time.monotonic):
        self.config_path = config_path
        self.clock = clock
        self.config = parse_config(DEFAULT_CONFIG)
        self.mtime_ns = None
        self.next_check = 0.0
        self.current(force=True)

    def current(self, force=False):
        now = self.clock()
        if not force and now < self.next_check:
            return self.config, False
        self.next_check = now + CONFIG_POLL_SECONDS
        try:
            mtime_ns = os.stat(self.config_path).st_mtime_ns
        except OSError:
            return self.config, False
        if not force and mtime_ns == self.mtime_ns:
            return self.config, False
        try:
            candidate = read_config(self.config_path)
        except (OSError, UnicodeError, json.JSONDecodeError, ValueError, TypeError):
            return self.config, False
        changed = candidate != self.config
        self.config = candidate
        self.mtime_ns = mtime_ns
        return self.config, changed


def sanitize_transcript(value):
    if not isinstance(value, str):
        return ""
    printable = "".join(
        character
        for character in value
        if character.isprintable() or character.isspace()
    )
    return " ".join(printable.split())[:MAX_TRANSCRIPT_CHARACTERS]


def emit(kind, transcript=""):
    safe_kind = kind if kind in ALLOWED_EVENT_KINDS else "error"
    safe_transcript = sanitize_transcript(transcript)
    print(
        json.dumps(
            {"kind": safe_kind, "transcript": safe_transcript},
            separators=(",", ":"),
        ),
        flush=True,
    )


def begins_with_phrase(text, phrases):
    return any(text == phrase or text.startswith(f"{phrase} ") for phrase in phrases)


def wake_variants(phrase):
    if phrase == "lucy":
        return ("lucy", "loosey", "lucie")
    return (phrase,)


def wake_recognizer(kaldi_recognizer, model, wake_phrases):
    grammar = json.dumps(
        [
            *(
                variant
                for phrase in wake_phrases
                for variant in wake_variants(phrase)
            ),
            "[unk]",
        ]
    )
    return kaldi_recognizer(model, SAMPLE_RATE, grammar)


def recognizer_text(recognizer, accepted):
    try:
        payload = json.loads(
            recognizer.Result() if accepted else recognizer.PartialResult()
        )
    except (AttributeError, json.JSONDecodeError, TypeError):
        return ""
    key = "text" if accepted else "partial"
    return sanitize_transcript(payload.get(key, "")).lower()


def recognizer_final_text(recognizer):
    try:
        payload = json.loads(recognizer.FinalResult())
    except (AttributeError, json.JSONDecodeError, TypeError):
        return ""
    return sanitize_transcript(payload.get("text", "")).lower()


def iter_exact_frames(stream, frame_bytes=FRAME_BYTES):
    pending = bytearray()
    while True:
        chunk = stream.read(frame_bytes - len(pending))
        if not chunk:
            return
        pending.extend(chunk)
        if len(pending) == frame_bytes:
            yield bytes(pending)
            pending.clear()


class UtteranceCapture:
    def __init__(
        self,
        preroll_frames=PREROLL_FRAMES,
        maximum_bytes=MAX_UTTERANCE_BYTES,
    ):
        self.preroll = deque(maxlen=preroll_frames)
        self.maximum_bytes = maximum_bytes
        self.audio = bytearray()
        self.started = False

    def observe(self, frame, speech_detected):
        self.preroll.append(frame)
        started_now = speech_detected and not self.started
        if started_now:
            self.started = True
            self.audio.extend(b"".join(self.preroll))
        elif self.started:
            self.audio.extend(frame)
        return started_now

    def at_limit(self):
        return len(self.audio) >= self.maximum_bytes

    def finish(self):
        completed = bytes(self.audio[: self.maximum_bytes])
        self.reset()
        return completed

    def reset(self):
        self.preroll.clear()
        self.audio.clear()
        self.started = False


def _read_bounded(stream, limit, destination):
    retained = bytearray()
    try:
        while True:
            chunk = stream.read(4_096)
            if not chunk:
                break
            remaining = limit - len(retained)
            if remaining > 0:
                retained.extend(chunk[:remaining])
    finally:
        destination.append(bytes(retained))
        stream.close()


def run_bounded(command, timeout_seconds):
    process = subprocess.Popen(
        command,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    stdout_result = []
    stderr_result = []
    stdout_thread = threading.Thread(
        target=_read_bounded,
        args=(process.stdout, MAX_PROCESS_OUTPUT_BYTES, stdout_result),
        daemon=True,
    )
    stderr_thread = threading.Thread(
        target=_read_bounded,
        args=(process.stderr, MAX_PROCESS_OUTPUT_BYTES, stderr_result),
        daemon=True,
    )
    stdout_thread.start()
    stderr_thread.start()
    timed_out = False
    try:
        process.wait(timeout=timeout_seconds)
    except subprocess.TimeoutExpired:
        timed_out = True
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    stdout_thread.join(timeout=2)
    stderr_thread.join(timeout=2)
    stdout = stdout_result[0] if stdout_result else b""
    stderr = stderr_result[0] if stderr_result else b""
    return process.returncode, stdout, stderr, timed_out


def parse_whisper_output(output):
    lines = output.decode("utf-8", errors="replace").splitlines()
    transcript = " ".join(
        line.split("]", 1)[-1].strip()
        for line in lines
        if line.strip() and not line.lstrip().startswith("whisper_")
    )
    return sanitize_transcript(transcript).lower()


def ensure_private_runtime_directory(runtime_directory):
    if os.path.lexists(runtime_directory) and os.path.islink(runtime_directory):
        raise OSError("unsafe runtime directory")
    os.makedirs(runtime_directory, mode=0o700, exist_ok=True)
    os.chmod(runtime_directory, 0o700)


def transcribe_with_whisper(binary, model, audio, runtime_directory):
    if (
        not binary
        or not model
        or not audio
        or not os.path.isfile(binary)
        or not os.access(binary, os.X_OK)
        or not os.path.isfile(model)
    ):
        return ""
    audio_path = None
    try:
        ensure_private_runtime_directory(runtime_directory)
        descriptor, audio_path = tempfile.mkstemp(
            prefix="utterance-",
            suffix=".wav",
            dir=runtime_directory,
        )
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb") as raw_file:
            with wave.open(raw_file, "wb") as output:
                output.setnchannels(1)
                output.setsampwidth(SAMPLE_WIDTH)
                output.setframerate(SAMPLE_RATE)
                output.writeframes(audio)
        return_code, stdout, _stderr, timed_out = run_bounded(
            [binary, "-m", model, "-f", audio_path, "-nt", "-np"],
            timeout_seconds=45,
        )
        if timed_out or return_code != 0:
            return ""
        return parse_whisper_output(stdout)
    except OSError:
        return ""
    finally:
        if audio_path:
            try:
                os.unlink(audio_path)
            except OSError:
                pass


def terminate_process(process):
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def main():
    if len(sys.argv) != 6:
        raise SystemExit(
            "Expected the Vosk model, configuration, optional Whisper binary and model, and private runtime directory."
        )

    from vosk import KaldiRecognizer, Model

    model = Model(sys.argv[1])
    config_reloader = ConfigReloader(sys.argv[2])
    config, _changed = config_reloader.current(force=True)
    whisper_binary = sys.argv[3]
    whisper_model = sys.argv[4]
    runtime_directory = sys.argv[5]
    passive_recognizer = wake_recognizer(
        KaldiRecognizer, model, config["wakePhrases"]
    )
    active_recognizer = KaldiRecognizer(model, SAMPLE_RATE)
    capture = UtteranceCapture()
    active = False

    recorder = subprocess.Popen(
        [
            "pw-record",
            "--raw",
            "--format",
            "s16",
            "--rate",
            str(SAMPLE_RATE),
            "--channels",
            "1",
            "-",
        ],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if recorder.stdout is None or recorder.stderr is None:
        terminate_process(recorder)
        raise RuntimeError("PipeWire streams are unavailable")
    recorder_diagnostics = []
    diagnostics_thread = threading.Thread(
        target=_read_bounded,
        args=(recorder.stderr, 4_096, recorder_diagnostics),
        daemon=True,
    )
    diagnostics_thread.start()
    emit("ready")
    try:
        for frame in iter_exact_frames(recorder.stdout):
            next_config, changed = config_reloader.current()
            if changed:
                config = next_config
                passive_recognizer = wake_recognizer(
                    KaldiRecognizer, model, config["wakePhrases"]
                )
                active_recognizer = KaldiRecognizer(model, SAMPLE_RATE)
                capture.reset()

            if not active:
                accepted = passive_recognizer.AcceptWaveform(frame)
                heard = recognizer_text(passive_recognizer, accepted)
                expected = {
                    variant
                    for phrase in config["wakePhrases"]
                    for variant in wake_variants(phrase)
                }
                if heard not in expected:
                    continue
                active = True
                active_recognizer = KaldiRecognizer(model, SAMPLE_RATE)
                capture.reset()
                emit("activated", config["wakePhrases"][0])
                continue

            accepted = active_recognizer.AcceptWaveform(frame)
            base_text = recognizer_text(active_recognizer, accepted)
            if capture.observe(frame, bool(base_text)) and not accepted:
                emit("listening")
            if capture.at_limit() and not accepted:
                base_text = recognizer_final_text(active_recognizer)
                accepted = True
            if not accepted:
                continue

            completed_audio = capture.finish()
            if not base_text:
                active_recognizer = KaldiRecognizer(model, SAMPLE_RATE)
                continue
            command_text = base_text
            if (
                whisper_binary
                and whisper_model
                and os.path.isfile(whisper_binary)
                and os.path.isfile(whisper_model)
            ):
                emit("transcribing")
                high_accuracy_text = transcribe_with_whisper(
                    whisper_binary,
                    whisper_model,
                    completed_audio,
                    runtime_directory,
                )
                if high_accuracy_text:
                    command_text = high_accuracy_text
                else:
                    emit(
                        "warning",
                        "High-accuracy transcription was unavailable; the local base transcript was used.",
                    )
            active_recognizer = KaldiRecognizer(model, SAMPLE_RATE)
            if command_text in VOICE_OFF_PHRASES:
                emit("off_requested", command_text)
                break
            if begins_with_phrase(command_text, config["deactivatePhrases"]):
                active = False
                passive_recognizer = wake_recognizer(
                    KaldiRecognizer, model, config["wakePhrases"]
                )
                emit("deactivated", command_text)
                continue
            emit("heard", command_text)
            emit("command", command_text)

        if recorder.poll() not in (None, 0):
            emit("error", "The PipeWire microphone stream stopped unexpectedly.")
    finally:
        terminate_process(recorder)
        diagnostics_thread.join(timeout=2)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError):
        emit("error", "The offline voice listener could not start safely.")
        raise SystemExit(1)
