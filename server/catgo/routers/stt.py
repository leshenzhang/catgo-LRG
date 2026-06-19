"""Local speech-to-text — pluggable native engine.

Desktop/Tauri voice dictation routes audio here instead of running Whisper in
the webview. On WebKit engines (WebKitGTK on Linux, WKWebView on macOS/iOS —
every Tauri webview except Windows' Chromium WebView2) the onnxruntime-web WASM
backend leaks ~0.8 GB of unreclaimable linear memory per inference, so a few
utterances OOM-kill the WebContent process (blank window). Native inference
avoids the webview entirely.

Two engines, selected by ``CATGO_STT_ENGINE``:

- ``faster-whisper`` (default): CTranslate2. CPU int8 everywhere, CUDA float16
  when an NVIDIA GPU is present. Self-contained pip dependency.
- ``whispercpp``: shells out to a whisper.cpp ``whisper-cli`` binary. Its value
  is the **Vulkan** backend, which runs on AMD/Intel iGPUs that CTranslate2
  (CUDA-only) cannot target. Requires a Vulkan-built ``whisper-cli`` and GGML
  models supplied by the host:
    CATGO_WHISPERCPP_BIN     path to whisper-cli (default: "whisper-cli" on PATH)
    CATGO_WHISPERCPP_MODELS  dir of ggml-<size>.bin (default ~/.cache/whisper.cpp/models)

The browser still runs the (tiny) Silero VAD and POSTs each finished segment as
raw little-endian float32 PCM at 16 kHz. Language post-processing (zh
Traditional→Simplified) stays on the client.
"""

import logging
import os
import shutil
import subprocess
import tempfile
import wave

import numpy as np
from fastapi import APIRouter, HTTPException, Query, Request
from pydantic import BaseModel

logger = logging.getLogger(__name__)

router = APIRouter(prefix="/stt", tags=["stt"])

# ─── Config ──────────────────────────────────────────────────────────────
_ENGINE = (os.environ.get("CATGO_STT_ENGINE") or "faster-whisper").strip().lower()
# CPU inference threads. faster-whisper/CTranslate2 defaults to a conservative 4;
# whisper.cpp uses this for -t too. Overridable via CATGO_STT_CPU_THREADS.
_CPU_THREADS = int(os.environ.get("CATGO_STT_CPU_THREADS") or min(8, os.cpu_count() or 4))
_WHISPERCPP_BIN = os.environ.get("CATGO_WHISPERCPP_BIN") or "whisper-cli"
_WHISPERCPP_MODELS = (
    os.environ.get("CATGO_WHISPERCPP_MODELS")
    or os.path.expanduser("~/.cache/whisper.cpp/models")
)

# Map the client's whisper model ids / aliases to a canonical size name shared by
# both engines (faster-whisper size and whisper.cpp ggml-<size>.bin).
_SIZE_ALIASES = {
    "tiny": "tiny",
    "tiny.en": "tiny.en",
    "base": "base",
    "base.en": "base.en",
    "small": "small",
    "small.en": "small.en",
    "medium": "medium",
    "medium.en": "medium.en",
    "large": "large-v3",
    "large-v2": "large-v2",
    "large-v3": "large-v3",
    "large-v3-turbo": "large-v3-turbo",
    "turbo": "large-v3-turbo",
}


def _resolve_size(model: str) -> str:
    """Turn a client model id (e.g. 'onnx-community/whisper-base') into a shared
    size name ('base'). Unknown ids fall back to 'base'."""
    name = (model or "").split("/")[-1].lower().replace("whisper-", "")
    return _SIZE_ALIASES.get(name, "base")


# ─── faster-whisper engine ───────────────────────────────────────────────
_model_cache: dict[str, object] = {}
_device_compute: tuple[str, str] | None = None


def _select_device() -> tuple[str, str]:
    """Probe for a usable CUDA device, else fall back to CPU int8."""
    try:
        import ctranslate2

        if ctranslate2.get_cuda_device_count() > 0:
            return "cuda", "float16"
    except Exception as exc:  # pragma: no cover - best-effort probe
        logger.debug("CUDA probe failed, using CPU: %s", exc)
    return "cpu", "int8"


def _build_model(size: str, device: str, compute_type: str):
    from faster_whisper import WhisperModel

    kwargs: dict = {"device": device, "compute_type": compute_type}
    if device == "cpu":
        kwargs["cpu_threads"] = _CPU_THREADS
    try:
        return WhisperModel(size, **kwargs)
    except Exception as exc:
        # First-run download pulls weights from HuggingFace, frequently blocked
        # (e.g. mainland China). Retry once via hf-mirror.com unless the user
        # pinned their own HF_ENDPOINT.
        if os.environ.get("HF_ENDPOINT"):
            raise
        logger.warning(
            "STT model %s download failed (%s); retrying via hf-mirror.com", size, exc
        )
        os.environ["HF_ENDPOINT"] = "https://hf-mirror.com"
        return WhisperModel(size, **kwargs)


def _get_model(size: str):
    global _device_compute
    if _device_compute is None:
        _device_compute = _select_device()
        logger.info("STT engine: faster-whisper device=%s compute=%s", *_device_compute)
    device, compute_type = _device_compute
    key = f"{size}:{device}"
    if key not in _model_cache:
        logger.info(
            "STT loading model %s (%s/%s, cpu_threads=%s)",
            size, device, compute_type, _CPU_THREADS if device == "cpu" else "-",
        )
        _model_cache[key] = _build_model(size, device, compute_type)
    return _model_cache[key]


def _transcribe_faster_whisper(audio: np.ndarray, lang: str | None, size: str):
    whisper = _get_model(size)
    segments, info = whisper.transcribe(audio, language=lang, beam_size=1, vad_filter=False)
    text = "".join(seg.text for seg in segments).strip()
    return text, getattr(info, "language", None)


# ─── whisper.cpp (Vulkan) engine ─────────────────────────────────────────
def _whispercpp_model_path(size: str) -> str:
    return os.path.join(_WHISPERCPP_MODELS, f"ggml-{size}.bin")


def _transcribe_whispercpp(audio: np.ndarray, lang: str | None, size: str):
    model_path = _whispercpp_model_path(size)
    if not os.path.exists(model_path):
        raise HTTPException(
            status_code=503,
            detail=f"whisper.cpp model missing: {model_path} "
            f"(download-ggml-model.sh {size})",
        )

    # whisper.cpp reads a 16 kHz mono PCM16 WAV file.
    pcm16 = np.clip(audio * 32767.0, -32768, 32767).astype("<i2")
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tf:
        wav_path = tf.name
    try:
        with wave.open(wav_path, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(16000)
            w.writeframes(pcm16.tobytes())

        cmd = [
            _WHISPERCPP_BIN, "-m", model_path, "-f", wav_path,
            "-nt",  # no timestamps → clean text on stdout
            "-t", str(_CPU_THREADS),
        ]
        if lang:
            cmd += ["-l", lang]
        try:
            proc = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        except FileNotFoundError:
            raise HTTPException(
                status_code=503, detail=f"whisper-cli not found: {_WHISPERCPP_BIN}"
            )
        if proc.returncode != 0:
            logger.error("whisper.cpp failed: %s", proc.stderr[-500:])
            raise HTTPException(status_code=500, detail="whisper.cpp transcription failed")
        text = " ".join(line.strip() for line in proc.stdout.splitlines() if line.strip())
        return text.strip(), lang
    finally:
        try:
            os.unlink(wav_path)
        except OSError:
            pass


# ─── API ─────────────────────────────────────────────────────────────────
class STTHealth(BaseModel):
    available: bool
    engine: str
    device: str
    compute_type: str


class STTResult(BaseModel):
    text: str
    language: str | None = None


@router.get("/health", response_model=STTHealth)
def health() -> STTHealth:
    """Report whether native STT is usable, which engine, and on what device.
    The client uses this to decide between backend STT and the WASM fallback."""
    if _ENGINE == "whispercpp":
        ok = bool(shutil.which(_WHISPERCPP_BIN) or os.path.exists(_WHISPERCPP_BIN))
        return STTHealth(
            available=ok, engine="whispercpp",
            device="vulkan/cpu", compute_type="ggml",
        )
    try:
        import faster_whisper  # noqa: F401

        device, compute_type = _select_device()
        return STTHealth(
            available=True, engine="faster-whisper",
            device=device, compute_type=compute_type,
        )
    except Exception as exc:
        logger.info("Native STT unavailable: %s", exc)
        return STTHealth(
            available=False, engine="faster-whisper",
            device="none", compute_type="none",
        )


@router.post("/transcribe", response_model=STTResult)
async def transcribe(
    request: Request,
    language: str = Query("en", description="BCP-47-ish tag; 'auto'/'' = detect"),
    model: str = Query("base", description="Client model id or size alias"),
) -> STTResult:
    """Transcribe a raw float32 PCM (16 kHz mono, little-endian) request body."""
    raw = await request.body()
    if not raw:
        raise HTTPException(status_code=400, detail="empty audio body")
    if len(raw) % 4 != 0:
        raise HTTPException(status_code=400, detail="body length not float32-aligned")

    audio = np.frombuffer(raw, dtype="<f4").astype(np.float32)
    if audio.size == 0:
        return STTResult(text="", language=None)

    size = _resolve_size(model)
    lang = None if language in ("", "auto") else language.split("-")[0]
    try:
        if _ENGINE == "whispercpp":
            # whisper.cpp wants a concrete language code; default to English.
            text, out_lang = _transcribe_whispercpp(audio, lang or "en", size)
        else:
            text, out_lang = _transcribe_faster_whisper(audio, lang, size)
    except HTTPException:
        raise
    except Exception as exc:
        logger.exception("STT transcription failed")
        raise HTTPException(status_code=500, detail=f"transcription failed: {exc}")

    return STTResult(text=text, language=out_lang)
