import json
import os
import subprocess
import sys
import tempfile
import wave
from collections import deque

import numpy as np
import torch
from openwakeword.model import Model as OpenWakeWordModel
from silero_vad import VADIterator, load_silero_vad
from vosk import KaldiRecognizer, Model

COMMANDS = {
    "firefox",
    "open firefox",
    "dolphin",
    "open dolphin",
    "system settings",
    "open system settings",
    "settings",
    "open settings",
    "code",
    "open code",
    "visual studio code",
    "open visual studio code",
    "close firefox",
    "close dolphin",
    "close system settings",
    "close settings",
    "close code",
    "close visual studio code",
    "move left",
    "move right",
    "move up",
    "move down",
    "click",
    "double click",
    "scroll up",
    "scroll down",
}

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


def normalize_phrase(value, fallback):
    phrase = value.strip().lower() if isinstance(value, str) else ""
    return phrase if phrase else fallback


def phrase_variants(value, fallback):
    phrases = [phrase.strip() for phrase in normalize_phrase(value, fallback).split(",")]
    return [phrase for phrase in phrases if phrase]


def load_config(config_path):
    try:
        with open(config_path, encoding="utf-8") as config_file:
            saved = json.load(config_file)
    except (OSError, json.JSONDecodeError):
        saved = {}
    return {
        "wakePhrases": phrase_variants(saved.get("wakePhrase"), DEFAULT_CONFIG["wakePhrase"]),
        "deactivatePhrases": phrase_variants(saved.get("deactivatePhrase"), DEFAULT_CONFIG["deactivatePhrase"]),
        "openPhrases": [normalize_phrase(item, "") for item in saved.get("openPhrases", DEFAULT_CONFIG["openPhrases"]) if normalize_phrase(item, "")],
        "closePhrases": [normalize_phrase(item, "") for item in saved.get("closePhrases", DEFAULT_CONFIG["closePhrases"]) if normalize_phrase(item, "")],
    }


def emit(kind, transcript=""):
    print(json.dumps({"kind": kind, "transcript": transcript}), flush=True)


def begins_with_phrase(text, phrases):
    return any(text == phrase or text.startswith(f"{phrase} ") for phrase in phrases)


def wake_variants(phrase):
    if phrase == "lucy":
        return ("lucy", "loosey", "lucie")
    return (phrase,)


def wake_recognizer(model, wake_phrases):
    # [unk] absorbs unrelated speech so passive mode cannot coerce it into a wake word.
    grammar = json.dumps([
        *(variant for phrase in wake_phrases for variant in wake_variants(phrase)),
        "[unk]",
    ])
    return KaldiRecognizer(model, 16000, grammar)


def openwakeword_model(config_path):
    model_path = os.path.join(os.path.dirname(config_path), "models", "lucy.onnx")
    if not os.path.isfile(model_path):
        return None
    return OpenWakeWordModel(wakeword_models=[model_path], inference_framework="onnx")


def wake_detected(detector, audio):
    if not detector:
        return False
    scores = detector.predict(np.frombuffer(audio, dtype=np.int16))
    return any(score >= 0.5 for score in scores.values())


def vad_event(detector, audio):
    samples = np.frombuffer(audio, dtype=np.int16).astype(np.float32) / 32768.0
    return detector(torch.from_numpy(samples), return_seconds=False)


def transcribe_with_whisper(binary, model, audio):
    if not binary or not model or not audio:
        return "", "The local speech engine did not receive audio."
    if not os.path.isfile(binary) or not os.path.isfile(model):
        return "", "The Whisper binary or speech model is missing. Reinstall high-accuracy voice."
    try:
        with tempfile.TemporaryDirectory(prefix="lucy-whisper-") as directory:
            audio_file = os.path.join(directory, "utterance.wav")
            with wave.open(audio_file, "wb") as output:
                output.setnchannels(1)
                output.setsampwidth(2)
                output.setframerate(16000)
                output.writeframes(audio)
            result = subprocess.run(
                [binary, "-m", model, "-f", audio_file, "-nt", "-np"],
                capture_output=True,
                text=True,
                timeout=45,
                check=False,
            )
    except subprocess.TimeoutExpired:
        return "", "Speech transcription timed out. Try a shorter command."
    except OSError as error:
        return "", f"Could not start the local speech engine: {error}"
    if result.returncode != 0:
        detail = result.stderr.strip() or "the local speech engine exited unexpectedly"
        return "", f"Speech transcription failed: {detail}"
    lines = [line.strip() for line in result.stdout.splitlines()]
    transcript = " ".join(
        line.split("]", 1)[-1].strip()
        for line in lines
        if line and not line.startswith("whisper_")
    )
    return " ".join(transcript.lower().split()), ""


def main():
    if len(sys.argv) != 5:
        raise SystemExit("Expected the Vosk model directory, configuration file, Whisper binary, and Whisper model.")

    model = Model(sys.argv[1])
    config_path = sys.argv[2]
    whisper_binary = sys.argv[3]
    whisper_model = sys.argv[4]
    config = load_config(config_path)
    recognizer = wake_recognizer(model, config["wakePhrases"])
    wake_detector = openwakeword_model(config_path)
    voice_activity = load_silero_vad()
    vad_iterator = VADIterator(voice_activity, sampling_rate=16000, threshold=0.5)
    active = False
    wake_audio = bytearray()
    utterance_audio = bytearray()
    utterance_started = False
    speech_preroll = deque(maxlen=16)
    try:
        recorder = subprocess.Popen(
            [
                "pw-record",
                "--raw",
                "--format",
                "s16",
                "--rate",
                "16000",
                "--channels",
                "1",
                "-",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except OSError as error:
        emit("error", f"Lucy could not start PipeWire recording: {error}")
        return
    if recorder.stdout is None:
        emit("error", "Lucy could not read microphone audio from PipeWire.")
        return
    emit("ready")
    try:
        while True:
            audio = recorder.stdout.read(1024)
            if not audio:
                detail = recorder.stderr.read().decode(errors="replace").strip() if recorder.stderr else ""
                emit("error", detail or "Lucy lost its PipeWire microphone stream.")
                break

            next_config = load_config(config_path)
            if next_config != config:
                config = next_config
                recognizer = wake_recognizer(model, config["wakePhrases"])
                wake_detector = openwakeword_model(config_path)
                vad_iterator.reset_states()
                wake_audio.clear()
                utterance_audio.clear()
                utterance_started = False
                speech_preroll.clear()
                continue

            if not active:
                wake_audio.extend(audio)
                while len(wake_audio) >= 2560:
                    wake_frame = bytes(wake_audio[:2560])
                    del wake_audio[:2560]
                    detected = wake_detected(wake_detector, wake_frame)
                    if not detected and recognizer.AcceptWaveform(wake_frame):
                        heard = json.loads(recognizer.Result()).get("text", "").strip()
                        detected = heard in {
                            variant
                            for phrase in config["wakePhrases"]
                            for variant in wake_variants(phrase)
                        }
                    if not detected:
                        continue
                    active = True
                    vad_iterator.reset_states()
                    utterance_audio.clear()
                    utterance_started = False
                    speech_preroll.clear()
                    emit("activated", config["wakePhrases"][0])
                    break
                continue

            speech_preroll.append(audio)
            event = vad_event(vad_iterator, audio)
            if event and "start" in event:
                utterance_started = True
                utterance_audio = bytearray(b"".join(speech_preroll))
                emit("listening")
            if utterance_started:
                utterance_audio.extend(audio)
            if len(utterance_audio) > 16_000 * 2 * 20:
                event = {"end": 0}
            if not event or "end" not in event or not utterance_started:
                continue
            completed_audio = bytes(utterance_audio)
            utterance_audio.clear()
            utterance_started = False
            speech_preroll.clear()
            command_text, transcription_error = transcribe_with_whisper(
                whisper_binary,
                whisper_model,
                completed_audio,
            )
            if not command_text:
                emit("error", transcription_error or "Lucy could not transcribe that command.")
                continue
            if command_text in VOICE_OFF_PHRASES:
                emit("off_requested", command_text)
                break
            if command_text in config["deactivatePhrases"]:
                active = False
                vad_iterator.reset_states()
                speech_preroll.clear()
                emit("deactivated", command_text)
                continue
            emit("heard", command_text)
            emit("command", command_text)
    finally:
        if recorder.poll() is None:
            recorder.terminate()
        try:
            recorder.wait(timeout=5)
        except subprocess.TimeoutExpired:
            recorder.kill()
            recorder.wait()


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        emit("error", f"Lucy could not start: {error}")
        raise SystemExit(1)