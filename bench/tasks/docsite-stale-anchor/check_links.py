"""Every README link into docs/install.md with an anchor lands on a real heading.

Independent of stilltrue: headings are slugged the way GitHub renders them (lowercase,
punctuation dropped, spaces to hyphens). Exit 0 when every such link resolves and at
least one exists.
"""

import re
import sys
from pathlib import Path


def slug(heading: str) -> str:
    text = re.sub(r"[^\w\- ]", "", heading.strip().lower())
    return text.replace(" ", "-")


headings = {
    slug(m.group(1))
    for m in re.finditer(r"^#+\s+(.+)$", Path("docs/install.md").read_text(), re.M)
}
anchors = re.findall(r"\(docs/install\.md#([^)\s]+)\)", Path("README.md").read_text())
if not anchors:
    sys.exit("README.md no longer links into docs/install.md")
broken = [a for a in anchors if a not in headings]
if broken:
    sys.exit(f"broken anchors: {broken}")
