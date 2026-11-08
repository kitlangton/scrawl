"""Evidence results use Hermes's native multimodal tool envelope."""
from __future__ import annotations

import base64
import io
import os
from pathlib import Path
import stat

from PIL import Image

from .process import AdapterError


def read_image(root: Path, reference: str) -> bytes:
    parts = reference.split("/")
    if not reference or any(p in ("", ".", "..") for p in parts) or "\\" in reference:
        raise AdapterError("invalid_image", "Image reference must stay inside the selected bundle.")
    fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in parts[:-1]:
            next_fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        image_fd = os.open(parts[-1], os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW, dir_fd=fd)
        with os.fdopen(image_fd, "rb") as stream:
            info = os.fstat(stream.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_size > 8 * 1024 * 1024:
                raise AdapterError("invalid_image", "Image is not a regular file within the 8 MiB limit.")
            data = stream.read(8 * 1024 * 1024 + 1)
            if len(data) > 8 * 1024 * 1024:
                raise AdapterError("invalid_image", "Image grew beyond the 8 MiB limit.")
    finally:
        os.close(fd)
    with Image.open(io.BytesIO(data)) as image:
        if image.format not in ("PNG", "JPEG") or image.width * image.height > 25_000_000:
            raise AdapterError("invalid_image", "Unsupported image format or dimensions.")
        image.load()
        image.thumbnail((1600, 1600))
        output = io.BytesIO()
        image.convert("RGB").save(output, format="JPEG", quality=85)
        return output.getvalue()


def result(text: str, root: Path, reference: str | None, *, temporary=False) -> dict | str:
    fallback = text
    if not reference:
        return text + "\nVisual evidence unavailable. Act on clear transcript instructions; explain any remaining blocker."
    try:
        data = read_image(root, reference)
    except (OSError, ValueError, AdapterError, Image.DecompressionBombError) as exc:
        return text + "\nVisual evidence unavailable: " + str(exc) + "\nAct on clear instructions; explain unresolved visual uncertainty."
    if temporary:
        fallback += "\nImage pixels are embedded in this result. If this model cannot view them, fetch the original Connect ID before using a separate vision tool; snapshot paths do not persist."
    else:
        path = str(root / reference)
        fallback += f"\nImage: {path}\nIf pixels are not attached on this model, use Hermes vision tools to inspect this image."
    return {"_multimodal": True, "content": [
        {"type": "text", "text": fallback},
        {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64," + base64.b64encode(data).decode("ascii")}},
    ], "text_summary": fallback}
