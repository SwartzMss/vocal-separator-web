import argparse
import json
import logging
import os
import shutil
import sys
import tempfile
from pathlib import Path

from separation import ALLOWED_EXTENSIONS, run_demucs

REPO_DIR = Path(__file__).resolve().parents[1]


def configure_logger() -> logging.Logger:
    log_dir = Path(os.getenv("LOG_DIR", REPO_DIR / "logs"))
    log_dir.mkdir(parents=True, exist_ok=True)
    log_file = Path(os.getenv("AGENT_LOG_FILE", log_dir / "agent.log"))

    handler = logging.FileHandler(log_file, encoding="utf-8")
    formatter = logging.Formatter("%(asctime)s %(levelname)s %(message)s")
    handler.setFormatter(formatter)

    logger = logging.getLogger("agent")
    logger.setLevel(logging.INFO)
    logger.handlers.clear()
    logger.addHandler(handler)
    logger.propagate = False
    return logger

def separate_to_directory(input_path: Path, output_dir: Path) -> dict[str, str]:
    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="demucs_agent_") as tmpdir:
        tmp_path = Path(tmpdir)
        vocals_tmp, instrumental_tmp = run_demucs(input_path, tmp_path)

        vocals_dest = output_dir / "vocals.wav"
        instrumental_dest = output_dir / "instrumental.wav"

        shutil.move(str(vocals_tmp), vocals_dest)
        shutil.move(str(instrumental_tmp), instrumental_dest)

    return {"vocals": str(vocals_dest), "instrumental": str(instrumental_dest)}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Headless Demucs agent used as a child process."
    )
    parser.add_argument("--input", required=True, help="Input audio file path.")
    parser.add_argument("--output-dir", required=True, help="Directory to store outputs.")
    parser.add_argument(
        "--json",
        action="store_true",
        help="Print JSON result to stdout (default). Deprecated flag for compatibility.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    logger = configure_logger()

    input_path = Path(args.input).expanduser().resolve()
    if not input_path.exists():
        logger.error("Input file not found: %s", input_path)
        print(json.dumps({"error": f"Input file not found: {input_path}"}), file=sys.stderr)
        sys.exit(1)

    suffix = input_path.suffix.lower()
    if suffix not in ALLOWED_EXTENSIONS:
        logger.error("Unsupported file type requested: %s", suffix)
        print(json.dumps({"error": f"Unsupported file type: {suffix}"}), file=sys.stderr)
        sys.exit(1)

    output_dir = Path(args.output_dir).expanduser().resolve()

    try:
        logger.info("Starting separation input=%s output_dir=%s", input_path, output_dir)
        result = separate_to_directory(input_path, output_dir)
    except RuntimeError as exc:
        logger.exception("Separation failed: %s", exc)
        print(json.dumps({"error": str(exc)}), file=sys.stderr)
        sys.exit(1)
    else:
        logger.info(
            "Separation completed vocals=%s instrumental=%s", result["vocals"], result["instrumental"]
        )
        print(json.dumps(result))


if __name__ == "__main__":
    main()
