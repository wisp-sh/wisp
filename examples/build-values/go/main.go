package main

import "errors"

type Packet struct {
	Label string
	Data []byte
	Sequence uint64
	Enabled bool
}

func Echo(value string) string { return value }
func Buffer(value []byte) []byte { return value }
func PacketValue(value Packet) Packet { return value }
func Checked(success bool) (Packet, error) {
	if !success { return Packet{}, errors.New("denied") }
	return Packet{Label: "hello 🦀", Data: []byte{0, 128, 255}, Sequence: 9007199254740994, Enabled: true}, nil
}
func main() {}

type Wide struct {
	A string; B string; C string
	D string; E string; F string
	G string; H string; I string
}
func WideValue(value Wide) Wide { return value }

type WispNote struct { Value string }
func Note(value WispNote) WispNote { return value }

// A named integer type and its constants define an enum. Numeric values remain
// ordinary Go values; the Component API exposes the case names.
type Mode uint8

const (
	ModeFast Mode = iota
	ModeCareful
)

func Labels(value []string) []string { return value }
func Selected(value *Mode) *Mode { return value }
func Modes(value []Mode) []Mode { return value }
func Choices(value []*Mode) []*Mode { return value }

type Batch struct {
	Labels []string
	Selected *Mode
	Modes []Mode
}

func BatchValue(value Batch) Batch { return value }
