"""Exported names for internal bone and control names, as src/naming.rs renders them.

The DCC checks are written against the internal (default) names; this lets them
check a rig exported with another naming convention too.
"""


def final(naming, internal):
    if not naming:
        return internal
    control = internal.endswith("_ctrl")
    tokens = internal.split("_")[:-1] if control else internal.split("_")
    at = max((i for i, t in enumerate(tokens) if t in ("l", "r") and i > 0), default=None)
    if at is None:
        name, side, desc = tokens, naming["center"], []
    else:
        name, side, desc = tokens[:at], naming["left" if tokens[at] == "l" else "right"], tokens[at + 1:]
    text = (naming["control"] if control else naming["joint"]).replace("{name}", "_".join(name))
    text = text.replace("{side}", side).replace("{desc}", "_".join(desc))
    return "_".join(t for t in text.split("_") if t)
