"""Local speech-to-text via faster-whisper (CTranslate2).

Desktop/Tauri voice dictation routes audio here instead of running Whisper in
the webview. On WebKit engines (WebKitGTK on Linux, WKWebView on macOS/iOS —
i.e. every Tauri webview except Windows' Chromium WebView2) the onnxruntime-web
WASM backend leaks ~0.8 GB of unreclaimable linear memory per inference, so a
few utterances OOM-kill the WebContent process (blank window). Native
CTranslate2 inference avoids the webview entirely: it is faster, uses the GPU
when a CUDA device is present, and costs the renderer nothing.

The browser still runs the (tiny) Silero VAD to segment speech; only the
finished audio segment is POSTed here as raw little-endian float32 PCM at
16 kHz. The response is plain text — language post-processing (e.g. zh
Traditional→Simplified) stays on the client so this endpoint is engine-only.
"""

import logging
import os

import numpy as np
from fastapi import APIRouter, HTTPException, Query, Request
from pydantic import BaseModel

logger = logging.getLogger(__name__)

router = APIRouter(prefix="/stt", tags=["stt"])

# CPU inference threads for CTranslate2. Its default is a conservative 4; on a
# many-core desktop that leaves the machine idle. Use up to 8 (Whisper is
# memory-bandwidth bound past that), overridable via CATGO_STT_CPU_THREADS.
_CPU_THREADS = int(os.environ.get("CATGO_STT_CPU_THREADS") or min(8, os.cpu_count() or 4))

# Lazily-built WhisperModel cache, keyed by "<size>:<device>". Building a model
# loads weights + allocates the CTranslate2 translator, so we keep one per size.
_model_cache: dict[str, object] = {}
# Resolved (device, compute_type) — probed once. cuda/float16 when a CUDA GPU is
# present, else cpu/int8 (fast, low-memory; works on any machine incl. iGPU,
# which CTranslate2 cannot target — CUDA only).
_device_compute: tuple[str, str] | None = None

# Map the client's whisper model ids / aliases to faster-whisper size names.
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


def _select_device() -> tuple[str, str]:
    """Probe for a usable CUDA device, else fall back to CPU int8."""
    try:
        import ctranslate2

        if ctranslate2.get_cuda_device_count() > 0:
            return "cuda", "float16"
    except Exception as exc:  # pragma: no cover - probe is best-effort
        logger.debug("CUDA probe failed, using CPU: %s", exc)
    return "cpu", "int8"


def _resolve_size(model: str) -> str:
    """Turn a client model id (e.g. 'onnx-community/whisper-base') into a
    faster-whisper size ('base'). Unknown ids fall back to 'base'."""
    name = (model or "").split("/")[-1].lower()
    name = name.replace("whisper-", "")
    return _SIZE_ALIASES.get(name, "base")


def _build_model(size: str, device: str, compute_type: str):
    from faster_whisper import WhisperModel

    kwargs: dict = {"device": device, "compute_type": compute_type}
    if device == "cpu":
        kwargs["cpu_threads"] = _CPU_THREADS
    try:
        return WhisperModel(size, **kwargs)
    except Exception as exc:
        # First-run download pulls weights from HuggingFace, frequently blocked
        # (e.g. mainland China). Retry once via the hf-mirror.com mirror, unless
        # the user pinned their own HF_ENDPOINT.
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
        logger.info("STT engine: device=%s compute=%s", *_device_compute)
    device, compute_type = _device_compute
    key = f"{size}:{device}"
    if key not in _model_cache:
        logger.info(
            "STT loading model %s (%s/%s, cpu_threads=%s)",
            size, device, compute_type, _CPU_THREADS if device == "cpu" else "-",
        )
        _model_cache[key] = _build_model(size, device, compute_type)
    return _model_cache[key]


class STTHealth(BaseModel):
    available: bool
    device: str
    compute_type: str


class STTResult(BaseModel):
    text: str
    language: str | None = None


@router.get("/health", response_model=STTHealth)
def health() -> STTHealth:
    """Report whether native STT is usable and on what device. The client uses
    this to decide between backend STT and the in-browser WASM fallback."""
    try:
        import faster_whisper  # noqa: F401

        device, compute_type = _select_device()
        return STTHealth(available=True, device=device, compute_type=compute_type)
    except Exception as exc:
        logger.info("Native STT unavailable: %s", exc)
        return STTHealth(available=False, device="none", compute_type="none")


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

    lang = None if language in ("", "auto") else language.split("-")[0]
    try:
        whisper = _get_model(_resolve_size(model))
        # beam_size=1 keeps dictation latency low; the VAD already trimmed silence.
        segments, info = whisper.transcribe(
            audio, language=lang, beam_size=1, vad_filter=False
        )
        text = "".join(seg.text for seg in segments).strip()
    except Exception as exc:
        logger.exception("STT transcription failed")
        raise HTTPException(status_code=500, detail=f"transcription failed: {exc}")

    return STTResult(text=text, language=getattr(info, "language", None))
