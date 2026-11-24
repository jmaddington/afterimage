"""Unpack a zip that may hold one .app bundle and nothing else.   python3 -I unpack_app.py <zip> <AppName.app> <empty dir>

The same parser validates and extracts, and every entry is judged before any is written.
"""
import os
import stat
import sys
import zipfile

MAX_BYTES = 500 << 20


def problem(infos, app):
    seen = set()
    for i in infos:
        name, parts = i.filename, i.filename.rstrip("/").split("/")
        kind = stat.S_IFMT(i.external_attr >> 16)
        if parts[0] != app or any(p in ("", ".", "..") for p in parts) or "\\" in name or "\0" in name or i.orig_filename != name:
            return f"unexpected path {name!r}"
        if kind not in (0, stat.S_IFREG, stat.S_IFDIR) or (kind == stat.S_IFDIR) != i.is_dir() and kind != 0:
            return f"not a plain file {name!r}"
        # macOS volumes ignore case: two spellings would be one file.
        key = name.rstrip("/").casefold()
        if key in seen:
            return f"duplicate {name!r}"
        seen.add(key)
    if sum(i.file_size for i in infos) > MAX_BYTES:
        return "too large"
    return None if seen else "empty"


def unpack(path, app, out):
    if os.listdir(out):
        raise ValueError("output folder isn't empty")
    with zipfile.ZipFile(path) as z:
        infos = z.infolist()
        if why := problem(infos, app):
            raise ValueError(why)
        for i in infos:
            # open() also refuses an entry whose local header disagrees with the directory.
            dest = os.path.join(out, *i.filename.rstrip("/").split("/"))
            if i.is_dir():
                os.makedirs(dest, exist_ok=True)
                continue
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            mode = 0o755 if (i.external_attr >> 16) & 0o111 else 0o644
            with z.open(i) as src, os.fdopen(os.open(dest, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode), "wb") as dst:
                while chunk := src.read(1 << 20):
                    dst.write(chunk)


if __name__ == "__main__":
    try:
        unpack(*sys.argv[1:4])
    except Exception as e:
        print(f"::error title=Build archive refused::{e}")
        sys.exit(1)
