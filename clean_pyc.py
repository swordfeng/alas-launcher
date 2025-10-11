import argparse
import os
from pathlib import Path
from typing import Iterable, Tuple


def find_pyc_files(root: Path) -> Iterable[Path]:
    """Yield all .pyc files under root (recursive)."""
    for dirpath, _, filenames in os.walk(root):
        for fn in filenames:
            if fn.endswith('.pyc'):
                yield Path(dirpath) / fn


def corresponding_py_for_pyc(pyc_path: Path) -> Path:
    """Return the Path to the likely corresponding .py source for a .pyc file.

    Rules:
    - If the .pyc is inside a __pycache__ directory, the source is one level up
      with the base module name (strip everything after the first dot in the
      pyc file name).
    - Otherwise, replace the .pyc suffix with .py in the same directory.
    """
    if pyc_path.parent.name == '__pycache__':
        # Example: __pycache__/module.cpython-38.opt-1.pyc -> ../module.py
        base = pyc_path.stem.split('.', 1)[0]
        return pyc_path.parent.parent / (base + '.py')
    else:
        return pyc_path.with_suffix('.py')


def clean_pyc(root: Path, dry_run: bool = True, verbose: bool = False) -> Tuple[int, int]:
    """Remove .pyc files that have corresponding .py sources.

    Returns a tuple (checked, removed).
    """
    checked = 0
    removed = 0
    for pyc in find_pyc_files(root):
        checked += 1
        src = corresponding_py_for_pyc(pyc)
        if src.exists():
            if verbose:
                print(f"Will remove: {pyc} (found source: {src})")
            if not dry_run:
                try:
                    pyc.unlink()
                    removed += 1
                except Exception as e:
                    print(f"Failed to remove {pyc}: {e}")
        else:
            if verbose:
                print(f"Keep: {pyc} (no source {src})")
    return checked, removed


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description='Delete .pyc files when corresponding .py sources exist.'
    )
    p.add_argument('path', nargs='?', default='.', help='Root path to scan')
    p.add_argument('--dry-run', action='store_true', help='Only show what would be deleted')
    p.add_argument('--verbose', action='store_true', help='Show verbose output')
    return p.parse_args()


def main() -> int:
    args = parse_args()
    root = Path(args.path).resolve()
    if not root.exists():
        print(f'Path does not exist: {root}')
        return 2

    dry_run = args.dry_run

    if args.verbose:
        print(f'Scanning: {root}')
        print(f'dry_run={dry_run}')

    checked, removed = clean_pyc(root, dry_run=dry_run, verbose=args.verbose)

    print(f"Checked .pyc files: {checked}")
    if dry_run:
        print(f"Dry run: would remove {removed} files")
    else:
        print(f"Removed {removed} files")

    return 0


if __name__ == '__main__':
    raise SystemExit(main())