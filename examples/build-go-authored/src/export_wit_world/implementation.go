package export_wit_world

import (
	"crypto/rand"
	"os"
	"runtime"
	"time"
)

func Add(a, b uint32) uint32    { return a + b }
func Negate(value bool) bool    { return !value }
func Widen(value uint64) uint64 { return value + 1 }
func EnvironmentCount() uint32  { return uint32(len(os.Environ())) }
func ClockReady() bool          { return time.Now().Unix() > 0 }

func Stress() uint32 {
	var answer uint32
	for n := 0; n < 8; n++ {
		data := make([]byte, 2<<20)
		data[len(data)-1] = 42
		answer = uint32(data[len(data)-1])
		runtime.KeepAlive(data)
		runtime.GC()
	}
	return answer
}

func RandomReady() bool {
	var data [8]byte
	_, err := rand.Read(data[:])
	return err == nil
}

func FilesystemDenied() bool {
	_, err := os.ReadFile("/etc/passwd")
	return err != nil
}
