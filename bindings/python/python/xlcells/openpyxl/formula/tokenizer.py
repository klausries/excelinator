"""Split an Excel formula into tokens, with the token types of openpyxl's tokenizer
(operands, functions, arrays, parentheses, separators, operators, white space).

    Tokenizer('=SUM(A1:B2)*2').items  # [FUNC OPEN 'SUM(', OPERAND RANGE 'A1:B2', ...]

Written from the rules of Eric Bachtal's formula tokenizer, which openpyxl also follows.
"""
import re

__all__ = ["Tokenizer", "Token", "TokenizerError"]


class TokenizerError(Exception):
    """A formula the tokenizer cannot split."""


class Token:
    """A piece of a formula: `value`, `type` and `subtype`."""
    __slots__ = ["value", "type", "subtype"]

    LITERAL = "LITERAL"
    OPERAND = "OPERAND"
    FUNC = "FUNC"
    ARRAY = "ARRAY"
    PAREN = "PAREN"
    SEP = "SEP"
    OP_PRE = "OPERATOR-PREFIX"
    OP_IN = "OPERATOR-INFIX"
    OP_POST = "OPERATOR-POSTFIX"
    WSPACE = "WHITE-SPACE"

    TEXT = "TEXT"
    NUMBER = "NUMBER"
    LOGICAL = "LOGICAL"
    ERROR = "ERROR"
    RANGE = "RANGE"
    OPEN = "OPEN"
    CLOSE = "CLOSE"
    ARG = "ARG"
    ROW = "ROW"

    def __init__(self, value, type_, subtype=""):
        self.value, self.type, self.subtype = value, type_, subtype

    def __repr__(self):
        return "{0} {1} {2}:".format(self.type, self.subtype, self.value)

    @classmethod
    def make_operand(cls, value):
        if value.startswith('"'):
            kind = cls.TEXT
        elif value.startswith("#"):
            kind = cls.ERROR
        elif value in ("TRUE", "FALSE"):
            kind = cls.LOGICAL
        else:
            try:
                float(value)
                kind = cls.NUMBER
            except ValueError:
                kind = cls.RANGE
        return cls(value, cls.OPERAND, kind)

    @classmethod
    def make_subexp(cls, value, func=False):
        closing = value[-1] in ")}"
        if func:
            kind = cls.FUNC
        elif value in ("{", "}"):
            kind = cls.ARRAY
        elif value in ("(", ")"):
            kind = cls.PAREN
        else:
            kind = cls.FUNC
        return cls(value, kind, cls.CLOSE if closing else cls.OPEN)

    def get_closer(self):
        return Token.make_subexp("}" if self.type == self.ARRAY else ")", func=self.type == self.FUNC)

    @classmethod
    def make_separator(cls, value):
        return cls(value, cls.SEP, cls.ARG if value == "," else cls.ROW)


_ERRORS = ("#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#GETTING_DATA")
_ENDERS = ",;}) +-*/^&=><%"
_EXPONENT = re.compile(r"^[1-9](\.[0-9]+)?[Ee]$")
_QUOTED = {'"': re.compile('"(?:[^"]*"")*[^"]*"(?!")'), "'": re.compile("'(?:[^']*'')*[^']*'(?!')")}


class Tokenizer:
    """The tokens of `formula` in `items`. Text that does not start with = is one literal."""

    def __init__(self, formula):
        self.formula = formula
        self.items = []
        self.token_stack = []
        self.offset = 0
        self.token = []
        self._parse()

    # --- the scan --------------------------------------------------------------
    def _parse(self):
        f = self.formula
        if self.offset or not f:
            return
        if f[0] != "=":
            self.items.append(Token(f, Token.LITERAL))
            return
        self.offset = 1
        while self.offset < len(f):
            ch = f[self.offset]
            # A sign inside a number such as 1.5E+3 belongs to it.
            if ch in "+-" and self.token and _EXPONENT.match("".join(self.token)):
                self.token.append(ch)
                self.offset += 1
                continue
            if ch in _ENDERS:
                self.save_token()
            step = self._step(ch)
            if step is None:
                self.token.append(ch)
                step = 1
            self.offset += step
        self.save_token()

    def _step(self, ch):
        if ch in "\"'":
            return self._string(ch)
        if ch == "[":
            return self._brackets()
        if ch == "#":
            return self._error()
        if ch in " \n":
            self.items.append(Token(ch, Token.WSPACE))
            return re.match(r"[ \n]+", self.formula[self.offset:]).end()
        if ch in "+-*/^&=><%":
            return self._operator()
        if ch in "{(":
            return self._open(ch)
        if ch in ")}":
            return self._close(ch)
        if ch in ";,":
            return self._separator(ch)
        return None

    def _expect_no_token(self, allowed=()):
        if self.token and self.token[-1] not in allowed:
            raise TokenizerError("Unexpected character at position {0} in '{1}'".format(self.offset, self.formula))

    def _string(self, quote):
        self._expect_no_token(allowed=":")
        m = _QUOTED[quote].match(self.formula[self.offset:])
        if m is None:
            raise TokenizerError("Reached end of formula while parsing {0} in {1}".format("string" if quote == '"' else "link", self.formula))
        text = m.group(0)
        if quote == '"':
            self.items.append(Token.make_operand(text))
        else:
            self.token.append(text)  # a quoted sheet name: part of the reference
        return len(text)

    def _brackets(self):
        depth = 0
        rest = self.formula[self.offset:]
        for i, ch in enumerate(rest):
            if ch == "[":
                depth += 1
            elif ch == "]":
                depth -= 1
                if depth == 0:
                    self.token.append(rest[:i + 1])
                    return i + 1
        raise TokenizerError("Encountered unmatched '[' in {0}".format(self.formula))

    def _error(self):
        self._expect_no_token(allowed="!")
        rest = self.formula[self.offset:]
        for code in _ERRORS:
            if rest.startswith(code):
                self.items.append(Token.make_operand("".join(self.token) + code))
                del self.token[:]
                return len(code)
        raise TokenizerError("Invalid error code at position {0} in '{1}'".format(self.offset, self.formula))

    def _operator(self):
        two = self.formula[self.offset:self.offset + 2]
        if two in (">=", "<=", "<>"):
            self.items.append(Token(two, Token.OP_IN))
            return 2
        ch = self.formula[self.offset]
        if ch == "%":
            token = Token("%", Token.OP_POST)
        elif ch in "*/^&=><":
            token = Token(ch, Token.OP_IN)
        else:
            before = next((t for t in reversed(self.items) if t.type != Token.WSPACE), None)
            infix = before is not None and (before.subtype == Token.CLOSE or before.type in (Token.OP_POST, Token.OPERAND))
            token = Token(ch, Token.OP_IN if infix else Token.OP_PRE)
        self.items.append(token)
        return 1

    def _open(self, ch):
        if ch == "{":
            self._expect_no_token()
            token = Token.make_subexp("{")
        elif self.token:
            token = Token.make_subexp("".join(self.token) + "(")
            del self.token[:]
        else:
            token = Token.make_subexp("(")
        self.items.append(token)
        self.token_stack.append(token)
        return 1

    def _close(self, ch):
        if not self.token_stack:
            raise IndexError("pop from empty list")
        token = self.token_stack.pop().get_closer()
        if token.value != ch:
            raise TokenizerError("Mismatched ( and { pair in '{0}'".format(self.formula))
        self.items.append(token)
        return 1

    def _separator(self, ch):
        if ch == ";":
            token = Token.make_separator(";")
        elif not self.token_stack or self.token_stack[-1].type == Token.PAREN:
            token = Token(",", Token.OP_IN)  # the union of references
        else:
            token = Token.make_separator(",")
        self.items.append(token)
        return 1

    # --- helpers scripts use -----------------------------------------------------
    def save_token(self):
        if self.token:
            self.items.append(Token.make_operand("".join(self.token)))
            del self.token[:]

    def check_scientific_notation(self):
        ch = self.formula[self.offset]
        if ch in "+-" and self.token and _EXPONENT.match("".join(self.token)):
            self.token.append(ch)
            self.offset += 1
            return True
        return False

    def assert_empty_token(self, can_follow=()):
        self._expect_no_token(can_follow)

    def render(self):
        if not self.items:
            return ""
        if self.items[0].type == Token.LITERAL:
            return self.items[0].value
        return "=" + "".join(t.value for t in self.items)
