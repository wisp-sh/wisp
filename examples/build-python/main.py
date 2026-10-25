"""A typed Python API. Wisp infers its Component contract inside guest CPython."""
from dataclasses import dataclass
from enum import Enum
from typing import Optional
from componentize_py_types import Result
import arithmetic_helpers as _helpers


@dataclass
class Person:
    name: str
    age: int


class Greeting(Enum):
    HELLO = 'hello'
    GOOD_DAY = 'good day'


def greet(name: str) -> str:
    return _helpers.greet(name)


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
