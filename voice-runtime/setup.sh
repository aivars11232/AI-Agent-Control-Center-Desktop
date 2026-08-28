#!/usr/bin/env bash

set -euo pipefail
umask 077

stage_dir="${VOICE_STAGE_DIR:?VOICE_STAGE_DIR must be set}"
cache_dir="${VOICE_CACHE_DIR:?VOICE_CACHE_DIR must be set}"
model_name="vosk-model-small-en-us-0.15"
model_archive="$cache_dir/$model_name.zip"
model_sha256="30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498"
model_bytes="41205931"
model_url="https://alphacephei.com/vosk/models/$model_name.zip"

if [[ "$stage_dir" != /* || "$cache_dir" != /* ]]; then
  echo "Voice installer paths must be absolute." >&2
  exit 1
fi

for command_name in python3 curl sha256sum stat unzip; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Missing required command: $command_name" >&2
    exit 1
  fi
done

python3 -c 'import platform, sys; raise SystemExit(0 if sys.version_info[:2] == (3, 14) and platform.machine() == "x86_64" else 1)' || {
  echo "Offline voice currently requires CPython 3.14 on x86_64 Linux." >&2
  exit 1
}

mkdir -p "$stage_dir" "$cache_dir" "$stage_dir/models"
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

cat >"$stage_dir/build-requirements.lock" <<'LOCK'
pip==26.2.1 \
    --hash=sha256:71138adf1f4ca900cdb7d289c21b7494329f2332b6d85f0e1c42108c0384ed3e
setuptools==84.0.0 \
    --hash=sha256:51a52592b3b99e102b609654876bd65f19f999935166d1352678931132b0c670
packaging==25.0 \
    --hash=sha256:29572ef2b1f17581046b3a2227d5c611fb25ec70ca1ba8554b24b0e69331a484
wheel==0.48.0 \
    --hash=sha256:3217dcc807155e45db462d7ef2431f5ddda0d7273b700d05a67b271ceb1287ab
LOCK

cat >"$stage_dir/runtime-requirements.lock" <<'LOCK'
vosk==0.3.45 \
    --hash=sha256:25e025093c4399d7278f543568ed8cc5460ac3a4bf48c23673ace1e25d26619f
cffi==2.1.1 \
    --hash=sha256:b0431303acaea1089ad4b3e9ce4e6518193def1118d4073ca848635ee4ea2e96
requests==2.34.2 \
    --hash=sha256:2a0d60c172f83ac6ab31e4554906c0f3b3588d37b5cb939b1c061f4907e278e0
tqdm==4.70.0 \
    --hash=sha256:7f585706bfddbdebf89daac705b2dfcc16890130727d3197ca62c732b4310953
srt==3.5.3 \
    --hash=sha256:4884315043a4f0740fd1f878ed6caa376ac06d70e135f306a6dc44632eed0cc0
websockets==17.1 \
    --hash=sha256:6f42912fa9eb4cb7c7ec9fde9b3332ba339eb8a8811981043d4029599f3d950b \
    --hash=sha256:f221081107b8c48184d99f7019604486376e7ef826037e70aad6b02540732c23
pycparser==3.0 \
    --hash=sha256:b727414169a36b7d524c1c3e31839a521725078d7b2ff038656844266160a992
charset-normalizer==3.5.1 \
    --hash=sha256:15f024313246a4ed976c60f440bb8d257815513a681d212ff74fd46f7d715a90 \
    --hash=sha256:6df0ec430f9a831772c23ca5a224cba36517a58a84bb32c32bb59a9fa67c47f6
idna==3.19 \
    --hash=sha256:815e7be7a7806d54abb586dc943addc79e8b2ee16915059658cbeff4b1b43bf4
urllib3==2.7.0 \
    --hash=sha256:9fb4c81ebbb1ce9531cce37674bbc6f1360472bc18ca9a553ede278ef7276897
certifi==2026.7.22 \
    --hash=sha256:62f22742b58a1a33014a2b6b706588a8d7e2a88ae7bd1a6ebe8c992928483775
LOCK

python3 -m venv "$stage_dir/venv"
"$stage_dir/venv/bin/python" -m pip install \
  --disable-pip-version-check \
  --cache-dir "$cache_dir/pip" \
  --require-hashes \
  --only-binary=:all: \
  --no-deps \
  --requirement "$stage_dir/build-requirements.lock"
"$stage_dir/venv/bin/python" -m pip install \
  --disable-pip-version-check \
  --cache-dir "$cache_dir/pip" \
  --require-hashes \
  --no-build-isolation \
  --requirement "$stage_dir/runtime-requirements.lock"

download_verified "$model_url" "$model_archive" "$model_sha256"
if [[ "$(stat --format='%s' "$model_archive")" != "$model_bytes" ]]; then
  echo "The verified Vosk archive has an unexpected size." >&2
  exit 1
fi
unzip -q "$model_archive" -d "$stage_dir/models"
if [[ ! -d "$stage_dir/models/$model_name" ]]; then
  echo "The Vosk archive did not contain the expected model directory." >&2
  exit 1
fi
"$stage_dir/venv/bin/python" -c 'import cffi, requests, vosk, websockets'

printf '%s\n' \
  '{"schemaVersion":1,"kind":"base","release":"base-v1","python":"3.14","architecture":"x86_64","voskVersion":"0.3.45","voskWheelSha256":"25e025093c4399d7278f543568ed8cc5460ac3a4bf48c23673ace1e25d26619f","model":"vosk-model-small-en-us-0.15","modelArchiveBytes":41205931,"modelArchiveSha256":"30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498"}' \
  >"$stage_dir/install-manifest.json"
