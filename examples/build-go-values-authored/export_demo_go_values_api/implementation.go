package export_demo_go_values_api

import (
	api "example.org/go-values/demo_go_values_api"
	witTypes "go.bytecodealliance.org/pkg/wit/types"
)

func Echo(value string) string                 { return value }
func Buffer(value []byte) []byte               { return value }
func PacketValue(value api.Packet) api.Packet  { return value }
func WideValue(value api.Wide) api.Wide        { return value }
func Note(value api.WispNote) api.WispNote     { return value }
func Labels(value []string) []string           { return value }
func Modes(value []api.Mode) []api.Mode         { return value }
func BatchValue(value api.Batch) api.Batch     { return value }
func Selected(value witTypes.Option[api.Mode]) witTypes.Option[api.Mode] {
	return value
}
func Choices(value []witTypes.Option[api.Mode]) []witTypes.Option[api.Mode] {
	return value
}
func ResultValue(value witTypes.Result[api.Packet, string]) witTypes.Result[api.Packet, string] {
	return value
}

func Checked(success bool) witTypes.Result[api.Packet, string] {
	if !success {
		return witTypes.Err[api.Packet, string]("denied")
	}
	return witTypes.Ok[api.Packet, string](api.Packet{
		Label: "hello 🦀", Data: []byte{0, 128, 255},
		Sequence: 9007199254740994, Enabled: true,
	})
}
