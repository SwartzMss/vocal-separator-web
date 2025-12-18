import asyncio
import os
import shutil
import uuid
from pathlib import Path

from fastapi import FastAPI, File, HTTPException, UploadFile
from fastapi.responses import FileResponse

from separation import ALLOWED_EXTENSIONS, run_demucs

# Base directory for storing job artefacts.
BASE_DIR = Path(__file__).resolve().parent
JOBS_DIR = BASE_DIR / "jobs"
JOBS_DIR.mkdir(exist_ok=True)

# Concurrency guard to avoid exhausting GPU memory.
MAX_CONCURRENT_JOBS = int(os.getenv("MAX_CONCURRENT_JOBS", "1"))
SEMAPHORE = asyncio.Semaphore(MAX_CONCURRENT_JOBS)

app = FastAPI(
    title="Vocal Separator",
    description="Demucs powered API for splitting tracks into vocals / instrumental.",
    version="0.1.0",
)


async def _write_upload(file: UploadFile, destination: Path) -> None:
    with destination.open("wb") as buffer:
        while True:
            data = await file.read(1024 * 1024)
            if not data:
                break
            buffer.write(data)
    await file.close()


@app.post("/api/jobs")
async def create_job(file: UploadFile = File(...)):
    if not file.filename:
        raise HTTPException(status_code=400, detail="File name missing.")
    suffix = Path(file.filename).suffix.lower()
    if suffix not in ALLOWED_EXTENSIONS:
        raise HTTPException(status_code=400, detail=f"Unsupported file type: {suffix}")

    job_id = uuid.uuid4().hex
    job_dir = JOBS_DIR / job_id
    job_dir.mkdir(parents=True, exist_ok=True)

    input_path = job_dir / f"input{suffix}"
    await _write_upload(file, input_path)

    try:
        async with SEMAPHORE:
            vocals_tmp, instrumental_tmp = await asyncio.to_thread(
                run_demucs, input_path, job_dir / "outputs"
            )

        vocals_path = job_dir / "vocals.wav"
        instrumental_path = job_dir / "instrumental.wav"
        shutil.move(str(vocals_tmp), vocals_path)
        shutil.move(str(instrumental_tmp), instrumental_path)
    except RuntimeError as exc:
        shutil.rmtree(job_dir, ignore_errors=True)
        raise HTTPException(status_code=500, detail=str(exc))
    except Exception:
        shutil.rmtree(job_dir, ignore_errors=True)
        raise

    return {
        "job_id": job_id,
        "instrumental_url": f"/api/jobs/{job_id}/instrumental",
        "vocals_url": f"/api/jobs/{job_id}/vocals",
    }


def _build_file_response(path: Path, filename: str) -> FileResponse:
    if not path.exists():
        raise HTTPException(status_code=404, detail="Job not found.")
    return FileResponse(path, media_type="audio/wav", filename=filename)


@app.get("/api/jobs/{job_id}/vocals")
async def get_vocals(job_id: str):
    path = JOBS_DIR / job_id / "vocals.wav"
    return _build_file_response(path, "vocals.wav")


@app.get("/api/jobs/{job_id}/instrumental")
async def get_instrumental(job_id: str):
    path = JOBS_DIR / job_id / "instrumental.wav"
    return _build_file_response(path, "instrumental.wav")
