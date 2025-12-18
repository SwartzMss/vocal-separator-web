import argparse
import shutil
import sys
import tempfile
from pathlib import Path

from separation import ALLOWED_EXTENSIONS, run_demucs


def separate_file(input_path: Path, output_dir: Path) -> tuple[Path, Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="demucs_cli_") as tmpdir:
        tmp_path = Path(tmpdir)
        vocals_tmp, instrumental_tmp = run_demucs(input_path, tmp_path)

        vocals_dest = output_dir / f"{input_path.stem}_vocals.wav"
        instrumental_dest = output_dir / f"{input_path.stem}_instrumental.wav"

        shutil.move(str(vocals_tmp), vocals_dest)
        shutil.move(str(instrumental_tmp), instrumental_dest)

    return vocals_dest, instrumental_dest


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Demucs CLI wrapper to split audio into vocals / instrumental."
    )
    parser.add_argument("input", help="Input audio file (.mp3/.wav/.m4a/.flac/.ogg/.aac).")
    parser.add_argument(
        "-o",
        "--output-dir",
        default="outputs",
        help="Directory to store generated wav files. Default: %(default)s",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    input_path = Path(args.input).expanduser().resolve()
    if not input_path.exists():
        print(f"Input file not found: {input_path}", file=sys.stderr)
        sys.exit(1)
    suffix = input_path.suffix.lower()
    if suffix not in ALLOWED_EXTENSIONS:
        print(f"Unsupported file type: {suffix}", file=sys.stderr)
        sys.exit(1)

    output_dir = Path(args.output_dir).expanduser().resolve()

    try:
        vocals_path, instrumental_path = separate_file(input_path, output_dir)
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(1)

    print(f"Vocals saved to: {vocals_path}")
    print(f"Instrumental saved to: {instrumental_path}")


if __name__ == "__main__":
    main()
