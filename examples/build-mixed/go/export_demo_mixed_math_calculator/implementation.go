package export_demo_mixed_math_calculator
import (
  arithmetic "example.org/mixed-math/demo_mixed_math_arithmetic"
  "example.org/mixed-math/internal/scale"
  witTypes "go.bytecodealliance.org/pkg/wit/types"
)
func DoubledSum(a, b uint32) uint32 { return scale.Double(arithmetic.Add(a,b)) }
func RichCheck() uint32 {
  values := arithmetic.Labels([]string{"hello", "世界"})
  if len(values)!=2 || values[0]!="hello" || values[1]!="世界" { return 1 }
  none := arithmetic.Selected(witTypes.None[arithmetic.Mode]())
  if !none.IsNone() { return 2 }
  selected := arithmetic.Selected(witTypes.Some[arithmetic.Mode](arithmetic.ModeCareful))
  if selected.IsNone() || selected.Some()!=arithmetic.ModeCareful {return 3}
  return 42
}
