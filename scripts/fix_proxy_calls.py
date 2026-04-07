#!/usr/bin/env python3
"""Fix proxy method calls to use keyword arguments after refactoring."""

import re
import sys
from pathlib import Path

# Methods that need template= keyword
TEMPLATE_METHODS = ['ask', 'think', 'reason']

# Methods with different keyword parameters
PARAM_METHODS = {
    'query_memory': 'query',
    'update_memory': 'data',
    'invoke': 'capability',
    'plan': 'goal',
    'reflect': 'trace_id',
    'communicate': ('target_agent', 'message'),  # two positional -> both keywords
    'branch': ('true_label', 'false_label'),  # condition_node is optional
    'verify': ('claim',),  # evidence is optional
    'const_': 'value',  # const_() also needs value= keyword
    'text': 'value',    # text() also needs value= keyword
    'string': 'value',  # string() also needs value= keyword
}

def fix_file(filepath: Path) -> bool:
    """Fix a single Python file. Returns True if changes were made."""
    content = filepath.read_text()
    original = content

    # Fix template methods: g.METHOD("name", "template") -> g.METHOD("name", template="template")
    for method in TEMPLATE_METHODS:
        # Match: g.METHOD("name", "template...") or g.METHOD("name", """template...""")
        # Positive lookbehind to ensure we're after g.METHOD( and first argument
        pattern = rf'(g\.{method}\([^,)]+,\s*)(["\'](?:[^"\'\\]|\\.)*["\']|"""(?:[^\\"]|\\.)*?"""|\'\'\'(?:[^\\"]|\\.)*?\'\'\')'

        def replace_template(match):
            prefix = match.group(1)
            template_str = match.group(2)
            return f'{prefix}template={template_str}'

        content = re.sub(pattern, replace_template, content)

    # Fix query_memory: g.query_memory("name", query="...") is already correct
    # But g.query_memory("name", "query") -> g.query_memory("name", query="query")
    # Also handle variables: g.const_("name", var) -> g.const_("name", value=var)
    for method, param in PARAM_METHODS.items():
        if isinstance(param, tuple):
            # Multiple params - more complex, skip for now
            continue

        # Match string literals OR identifiers (variables), but NOT if followed by =
        # This prevents matching already-keyworded args like capability="search"
        pattern = rf'(g\.{method}\([^,)]+,\s*)(["\'](?:[^"\'\\]|\\.)*["\']|"""(?:[^\\"]|\\.)*?"""|\'\'\'(?:[^\\"]|\\.)*?\'\'\'|\w+)(?!\s*=)'

        def replace_param(match):
            prefix = match.group(1)
            param_str = match.group(2)
            return f'{prefix}{param}={param_str}'

        content = re.sub(pattern, replace_param, content)

    # Fix communicate: g.communicate("name", "target", "message") -> g.communicate("name", target_agent="target", message="message")
    # This is trickier - need to match two positional string args
    pattern = r'(g\.communicate\([^,)]+,\s*)(["\'](?:[^"\'\\]|\\.)*["\'])\s*,\s*(["\'](?:[^"\'\\]|\\.)*["\']|"""(?:[^\\"]|\\.)*?"""|\'\'\'(?:[^\\"]|\\.)*?\'\'\')'

    def replace_communicate(match):
        prefix = match.group(1)
        target = match.group(2)
        message = match.group(3)
        return f'{prefix}target_agent={target}, message={message}'

    content = re.sub(pattern, replace_communicate, content)

    if content != original:
        filepath.write_text(content)
        return True
    return False

def main():
    root_dir = Path(__file__).parent.parent
    examples_dir = root_dir / 'examples' / 'python'
    skills_dir = root_dir / '.agents' / 'skills'

    if not examples_dir.exists():
        print(f"Error: {examples_dir} not found")
        return 1

    fixed_count = 0

    # Fix examples
    for py_file in examples_dir.rglob('*.py'):
        if fix_file(py_file):
            print(f"Fixed: {py_file.relative_to(root_dir)}")
            fixed_count += 1

    # Fix skills if they exist
    if skills_dir.exists():
        for py_file in skills_dir.rglob('*.py'):
            if fix_file(py_file):
                print(f"Fixed: {py_file.relative_to(root_dir)}")
                fixed_count += 1

    print(f"\nFixed {fixed_count} files")
    return 0

if __name__ == '__main__':
    sys.exit(main())
