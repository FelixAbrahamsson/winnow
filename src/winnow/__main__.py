"""Enable `python -m winnow` and serve as the PyInstaller entry point."""

import sys

from winnow.cli import main

if __name__ == "__main__":
    sys.exit(main())
