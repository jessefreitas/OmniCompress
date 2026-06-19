"""Make eval/ modules importable by pytest (flat, script-style layout)."""
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
