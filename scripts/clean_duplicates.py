#!/usr/bin/env python3
"""Remove duplicate keyword arguments like value=value=..."""

import re
from pathlib import Path

def clean_file(filepath: Path) -> bool:
    """Clean duplicate keywords in a file. Returns True if changes were made."""
    content = filepath.read_text()
    original = content

    # Pattern to match duplicate keywords like capability=capability=, value=value=, etc.
    pattern = r'\b(capability|query|value|data|template)=\1='

    # Keep replacing until no more matches
    while re.search(pattern, content):
        content = re.sub(pattern, r'\1=', content)

    if content != original:
        filepath.write_text(content)
        return True
    return False

def main():
    root = Path(__file__).parent.parent

    count = 0
    for py_file in (root / 'examples' / 'python').rglob('*.py'):
        if clean_file(py_file):
            print(f"Cleaned: {py_file.relative_to(root)}")
            count += 1

    for py_file in (root / '.agents' / 'skills').rglob('*.py'):
        if clean_file(py_file):
            print(f"Cleaned: {py_file.relative_to(root)}")
            count += 1

    print(f"\nCleaned {count} files")

if __name__ == '__main__':
    main()
