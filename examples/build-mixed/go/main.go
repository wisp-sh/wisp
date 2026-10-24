package main

import "example.org/mixed-math/internal/scale"

func ExportCalculatorDoubledSum(a, b uint32) uint32 {
	return scale.Double(ImportArithmeticAdd(a, b))
}
func main() {}
