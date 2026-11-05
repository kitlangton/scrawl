#!/usr/bin/env python3
"""Compatibility entrypoint; the TypeScript guard owns color policy."""
from pathlib import Path
import subprocess
import sys

if __name__ == '__main__':
    sys.exit(subprocess.call(['node', str(Path(__file__).with_suffix('.ts')), *sys.argv[1:]]))
