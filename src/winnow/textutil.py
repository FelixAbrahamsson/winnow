"""Turn URLs inside plain metadata values into clickable HTML links."""

from __future__ import annotations

import html as _html
import re

# http/https/ftp/file URLs, or bare www.* hosts.
_URL_RE = re.compile(r'((?:https?|ftp|file)://[^\s<>"\']+|www\.[^\s<>"\']+)')
# Trailing punctuation that is almost always sentence punctuation, not URL.
_TRAILING = ".,;:!?)]}>\"'"


def linkify(text: str) -> tuple[str, bool]:
    """Return (html, found_link). ``html`` has URLs wrapped in <a> tags and all
    other text HTML-escaped. ``found_link`` is False when there were no URLs."""
    out: list[str] = []
    last = 0
    found = False
    for m in _URL_RE.finditer(text):
        start = m.start()
        url = m.group(0)
        # peel trailing punctuation back out of the link
        stripped = url.rstrip(_TRAILING)
        trailing = url[len(stripped):]
        url = stripped
        if not url:
            continue
        out.append(_html.escape(text[last:start]))
        href = url if "://" in url else "https://" + url
        out.append(
            f'<a href="{_html.escape(href, quote=True)}">{_html.escape(url)}</a>'
        )
        out.append(_html.escape(trailing))
        last = m.end()
        found = True
    out.append(_html.escape(text[last:]))
    return "".join(out), found
