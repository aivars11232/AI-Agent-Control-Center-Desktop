#!/usr/bin/env bash

set -euo pipefail

runtime_dir="${VOICE_RUNTIME_DIR:?VOICE_RUNTIME_DIR must be set}"
venv_dir="$runtime_dir/venv"
model_dir="$runtime_dir/models/vosk-model-small-en-us-0.15"
model_archive="$runtime_dir/vosk-model-small-en-us-0.15.zip"

for command_name in python3 curl unzip; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Missing required command: $command_name" >&2
    exit 1
  fi
done

mkdir -p "$runtime_dir/models"
if [[ ! -x "$venv_dir/bin/python" ]]; then
  python3 -m venv "$venv_dir"
fi
"$venv_dir/bin/python" -m pip install --disable-pip-version-check --upgrade pip
"$venv_dir/bin/python" -m pip install --disable-pip-version-check --no-cache-dir --upgrade \
  --index-url https://download.pytorch.org/whl/cpu torch torchaudio
"$venv_dir/bin/python" -m pip install --disable-pip-version-check --no-cache-dir --upgrade \
  vosk numpy openwakeword
"$venv_dir/bin/python" -m pip install --disable-pip-version-check --no-cache-dir --upgrade \
  --no-deps silero-vad

if [[ ! -d "$model_dir" ]]; then
  curl --fail --location --retry 2 \
    --output "$model_archive" \
    "https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip"
  unzip -q "$model_archive" -d "$runtime_dir/models"
  rm -f "$model_archive"
fi

touch "$runtime_dir/.openwakeword-silero-ready"