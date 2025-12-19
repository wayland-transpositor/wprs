import pathlib
import runpy
import sys


def main() -> None:
    here = pathlib.Path(__file__).resolve().parent
    bundled = here / "wprs.py"
    if not bundled.exists():
        raise RuntimeError(
            "wprs.py is not bundled in this wheel; build via ./scripts/package.sh"
        )
    runpy.run_path(str(bundled), run_name="__main__")


if __name__ == "__main__":
    main()

