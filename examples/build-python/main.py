"""A typed Python API. Wisp infers its Component contract inside guest CPython."""
from typing import Optional
from componentize_py_types import Result
from models import Person, Greeting
from arithmetic_helpers.operations import greet
import arithmetic_helpers as _helpers


def total(values: list[int]) -> int:
    return _helpers.total(values)


def birthday(value: Person) -> Person:
    return Person(value.name, value.age + 1)


def checked(value: int) -> Result[int, str]:
    return _helpers.checked(value)


def echo(data: bytes) -> bytes:
    return data


def maybe_greet(name: Optional[str]) -> Optional[str]:
    return greet(name) if name is not None else None


def echo_greeting(value: Greeting) -> Greeting:
    return value
