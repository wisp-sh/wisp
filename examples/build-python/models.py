"""Public types can live in an imported module."""
from dataclasses import dataclass
from enum import Enum


@dataclass
class Person:
    name: str
    age: int


class Greeting(Enum):
    HELLO = 'hello'
    GOOD_DAY = 'good day'
