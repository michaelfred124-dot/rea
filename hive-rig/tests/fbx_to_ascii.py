"""Rewrite a binary FBX 7.x file as ASCII FBX 7.x, the way exporters write the
text form, to check hive-rig's ASCII reader on real files (Blender can only
write binary FBX).

    python3 -I fbx_to_ascii.py in.fbx out.fbx

Written from the public binary layout, independently of hive-rig's Rust
reader: records of a name, typed properties and nested records (64-bit
offsets from 7.5), arrays optionally zlib-compressed. In the text, objects are
named `Class::Name`, arrays are `*count { a: ... }`, bools `T`/`F`, raw bytes
base64 after an empty value. The input is untrusted: every length is checked
against the file.
"""

import base64
import struct
import sys
import zlib

MAGIC = b"Kaydara FBX Binary  \x00"


class Bad(Exception):
    pass


def read(data):
    if not data.startswith(MAGIC) or len(data) < 27:
        raise Bad("not a binary FBX")
    version = struct.unpack_from("<I", data, 23)[0]
    if not 7000 <= version < 8000:
        raise Bad("FBX version %d isn't 7.x" % version)
    wide = version >= 7500
    w, fmt = (8, "<Q") if wide else (4, "<I")

    def size(at):
        if at + w > len(data):
            raise Bad("cut short")
        return struct.unpack_from(fmt, data, at)[0]

    def value(at, limit):
        code = chr(data[at])
        at += 1

        def take(n):
            nonlocal at
            if at + n > limit:
                raise Bad("a property runs past its record")
            s = data[at : at + n]
            at += n
            return s

        scalars = {"C": ("<?", 1), "Y": ("<h", 2), "I": ("<i", 4), "L": ("<q", 8), "F": ("<f", 4), "D": ("<d", 8)}
        if code in scalars:
            f, n = scalars[code]
            v = struct.unpack(f, take(n))[0]
            return (code, v), at
        if code in "SR":
            n = struct.unpack("<I", take(4))[0]
            s = take(n)
            return (code, s), at
        arrays = {"f": ("f", 4), "d": ("d", 8), "l": ("q", 8), "i": ("i", 4), "b": ("?", 1)}
        if code in arrays:
            count, encoding, stored = struct.unpack("<III", take(12))
            f, n = arrays[code]
            raw = take(stored)
            if encoding == 1:
                d = zlib.decompressobj()
                raw = d.decompress(raw, count * n)
            elif encoding != 0:
                raise Bad("unknown array encoding")
            if len(raw) != count * n:
                raise Bad("an array has the wrong length")
            return (code, struct.unpack("<%d%s" % (count, f), raw)), at
        raise Bad("unknown property type %r" % code)

    def node(at, depth):
        if depth > 64:
            raise Bad("records nest too deeply")
        end, count, plen = size(at), size(at + w), size(at + 2 * w)
        if at + 3 * w >= len(data):
            raise Bad("cut short")
        nlen = data[at + 3 * w]
        if end == 0:
            return None, at + 3 * w + 1
        if not at < end <= len(data):
            raise Bad("a record ends outside the file")
        p = at + 3 * w + 1
        name = data[p : p + nlen].decode("utf-8", "replace")
        p += nlen
        pend = p + plen
        if pend > end:
            raise Bad("properties run past the record")
        props = []
        for _ in range(count):
            v, p = value(p, pend)
            props.append(v)
        if p != pend:
            raise Bad("property lengths don't add up")
        kids = []
        while p < end:
            k, p = node(p, depth + 1)
            if k is None:
                break
            kids.append(k)
        return (name, props, kids), end

    top, at = [], 27
    while at < len(data):
        n, at = node(at, 0)
        if n is None:
            break
        top.append(n)
    return version, top


def text(s):
    s = s.decode("utf-8", "replace")
    if "\x00\x01" in s:
        name, cls = s.split("\x00\x01", 1)
        s = cls + "::" + name
    return '"' + s.replace('"', "&quot;") + '"'


def number(v):
    if isinstance(v, bool):
        return str(int(v))
    if isinstance(v, float):
        return repr(v)
    return str(v)


def write(out, n, depth):
    name, props, kids = n
    pad = "\t" * depth
    parts = []
    for code, v in props:
        if code == "C":
            parts.append("T" if v else "F")
        elif code == "S":
            parts.append(text(v))
        elif code == "R":
            b = base64.b64encode(v).decode()
            parts.append(',\n%s\t"%s"' % (pad, b) if b else '""')
        elif code in "fdlib":
            vals = [number(x) for x in v]
            lines = [",".join(vals[i : i + 20]) for i in range(0, len(vals), 20)]
            parts.append("*%d {\n%s\ta: %s\n%s}" % (len(vals), pad, ",\n".join(lines), pad))
        else:
            parts.append(number(v))
    line = "%s%s: %s" % (pad, name, ", ".join(parts))
    # A raw value's leading comma already separates it.
    line = line.replace(", ,\n", ",\n")
    out.append(line)
    if kids:
        out[-1] += " {"
        for k in kids:
            write(out, k, depth + 1)
        out.append(pad + "}")


def main():
    src, dst = sys.argv[1], sys.argv[2]
    with open(src, "rb") as f:
        version, top = read(f.read())
    out = ["; FBX %d.%d.0 project file" % (version // 1000, version // 100 % 10), "; converted from binary", ""]
    for n in top:
        out.append("")
        out.append("; %s" % n[0])
        write(out, n, 0)
    with open(dst, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    print("wrote %s: ASCII FBX %d, %d top-level records" % (dst, version, len(top)))


if __name__ == "__main__":
    try:
        main()
    except Bad as e:
        sys.exit("fbx_to_ascii: %s" % e)
