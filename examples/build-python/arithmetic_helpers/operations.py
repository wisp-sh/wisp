from componentize_py_types import Err


def greet(name: str) -> str:
    return f'Hello, {name}!'


def total(values: list[int]) -> int:
    return sum(values)


def birthday(value):
    return type(value)(value.name, value.age + 1)


def checked(value: int) -> int:
    if value < 0:
        raise Err('value must be nonnegative')
    return value * 2
