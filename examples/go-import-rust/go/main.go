package main

import (
	"fmt"

	arithmetic "example.org/rust-arithmetic"
)

func DoubledSum(a, b uint32) uint32 {
	return arithmetic.Add(a, b) * 2
}

func Negate(value bool) bool {
	return arithmetic.Negate(value)
}

func Widen(value uint64) uint64 {
	return arithmetic.Widen(value)
}

func main() {
	fmt.Println(DoubledSum(20, 1))
}
