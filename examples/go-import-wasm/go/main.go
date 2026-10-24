package main

import (
	"fmt"

	arithmetic "example.org/cpp-arithmetic"
)

// DoubledSum is public, so Wisp exports it from the resulting package.
func DoubledSum(a, b uint32) uint32 {
	return arithmetic.Add(a, b) * 2
}

func main() {
	fmt.Println(DoubledSum(20, 1))
}
