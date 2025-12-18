import os
import subprocess
import sys
from pathlib import Path
from typing import Tuple

ALLOWED_EXTENSIONS = {".mp3", ".wav", ".m4a", ".flac", ".ogg", ".aac"}

DEMUCS_MODEL = os.getenv("DEMUCS_MODEL", "mdx_extra_q")
DEMUCS_DEVICE = os.getenv("DEMUCS_DEVICE", "cuda")


def run_demucs(input_path: Path, output_root: Path) -> Tuple[Path, Path]:
    """
    Execute Demucs CLI in two-stem mode and return generated wav paths.
    """
    output_root.mkdir(parents=True, exist_ok=True)
    cmd = [
        sys.executable,
        "-m",
        "demucs",
        "--two-stems=vocals",
        "-n",
        DEMUCS_MODEL,
        "-d",
        DEMUCS_DEVICE,
        "--out",
        str(output_root),
        str(input_path),
    ]
    completed = subprocess.run(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"Demucs failed with exit code {completed.returncode}:\n{completed.stdout}"
        )

    vocals_file = next(output_root.rglob("vocals.wav"), None)
    instrumental_file = next(output_root.rglob("no_vocals.wav"), None)
    if instrumental_file is None:
        instrumental_file = next(output_root.rglob("instrumental.wav"), None)

    if not vocals_file or not instrumental_file:
        raise RuntimeError("Unable to locate Demucs output files.")

    return vocals_file, instrumental_file
