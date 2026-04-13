class PhigError(Exception):
    def __init__(self, msg: str, pos: int | None = None):
        self.msg = msg
        self.pos = pos

    def __str__(self) -> str:
        if self.pos is not None:
            return f"at position {self.pos}: {self.msg}"
        return self.msg
