#!/usr/bin/env bash

set -euo pipefail
umask 077

stage_dir="${VOICE_STAGE_DIR:?VOICE_STAGE_DIR must be set}"
cache_dir="${VOICE_CACHE_DIR:?VOICE_CACHE_DIR must be set}"
whisper_commit="f049fff95a089aa9969deb009cdd4892b3e74916"
source_archive="$cache_dir/whisper.cpp-$whisper_commit.tar.gz"
source_sha256="279af4ce60dbf397362868f3bacc75b56a4332ac2541cae155070093f6aaf0e3"
source_url="https://github.com/ggml-org/whisper.cpp/archive/$whisper_commit.tar.gz"
model_file="$cache_dir/ggml-base.en.bin"
model_sha256="a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002"
model_bytes="147964211"
model_url="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"

if [[ "$stage_dir" != /* || "$cache_dir" != /* ]]; then
  echo "Voice installer paths must be absolute." >&2
  exit 1
fi

for command_name in cmake curl sha256sum stat tar; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Missing required command: $command_name" >&2
    exit 1
  fi
done

mkdir -p "$stage_dir" "$cache_dir" "$stage_dir/source"
chmod 0700 "$stage_dir" "$cache_dir"

download_verified() {
  local url="$1"
  local destination="$2"
  local expected_sha256="$3"
  local partial="$destination.part"
  if [[ -f "$destination" ]] && printf '%s  %s\n' "$expected_sha256" "$destination" | sha256sum --check --status; then
    return
  fi
  rm -f -- "$destination"
  curl --fail --location --retry 2 --continue-at - --output "$partial" "$url"
  if ! printf '%s  %s\n' "$expected_sha256" "$partial" | sha256sum --check --status; then
    rm -f -- "$partial"
    echo "Downloaded artifact checksum did not match." >&2
    return 1
  fi
  mv -f -- "$partial" "$destination"
}

download_verified "$source_url" "$source_archive" "$source_sha256"
download_verified "$model_url" "$model_file" "$model_sha256"
if [[ "$(stat --format='%s' "$model_file")" != "$model_bytes" ]]; then
  echo "The verified Whisper model has an unexpected size." >&2
  exit 1
fi

tar --extract --gzip --file "$source_archive" --directory "$stage_dir/source" --strip-components=1
cmake \
  -S "$stage_dir/source" \
  -B "$stage_dir/build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DWHISPER_BUILD_EXAMPLES=ON \
  -DWHISPER_BUILD_SERVER=OFF \
  -DWHISPER_BUILD_TESTS=OFF
cmake --build "$stage_dir/build" --config Release --parallel 2

candidate="$stage_dir/build/bin/whisper-cli"
if [[ ! -x "$candidate" ]]; then
  echo "The pinned whisper.cpp build did not produce whisper-cli." >&2
  exit 1
fi
install -Dm755 "$candidate" "$stage_dir/whisper-cli"
install -Dm600 "$model_file" "$stage_dir/ggml-base.en.bin"

printf '%s\n' \
  '{"schemaVersion":1,"kind":"high","release":"high-v1","whisperVersion":"1.9.1","whisperCommit":"f049fff95a089aa9969deb009cdd4892b3e74916","sourceArchiveSha256":"279af4ce60dbf397362868f3bacc75b56a4332ac2541cae155070093f6aaf0e3","model":"ggml-base.en.bin","modelBytes":147964211,"modelSha256":"a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002"}' \
  >"$stage_dir/install-manifest.json"
