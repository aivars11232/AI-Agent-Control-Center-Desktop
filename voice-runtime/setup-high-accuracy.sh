#!/usr/bin/env bash

set -euo pipefail

runtime_dir="${VOICE_RUNTIME_DIR:?VOICE_RUNTIME_DIR must be set}"
source_dir="$runtime_dir/whisper.cpp"
build_dir="$source_dir/build"
model_dir="$runtime_dir/models"
model_file="$model_dir/ggml-base.en.bin"
binary="$runtime_dir/whisper-cli"

for command_name in git cmake curl; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Missing required command: $command_name" >&2
    exit 1
  fi
done

mkdir -p "$model_dir"
if [[ ! -d "$source_dir/.git" ]]; then
  git clone --depth 1 https://github.com/ggml-org/whisper.cpp.git "$source_dir"
fi

cmake -S "$source_dir" -B "$build_dir" -DCMAKE_BUILD_TYPE=Release -DWHISPER_BUILD_EXAMPLES=ON
cmake --build "$build_dir" --config Release --parallel 2

if [[ ! -x "$binary" ]]; then
  candidate="$(find "$build_dir" -type f -name whisper-cli -perm -111 -print -quit)"
  if [[ -z "$candidate" ]]; then
    echo "whisper.cpp built without whisper-cli." >&2
    exit 1
  fi
  install -Dm755 "$candidate" "$binary"
fi

if [[ ! -f "$model_file" ]]; then
  curl --fail --location --retry 2 \
    --output "$model_file" \
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
fi