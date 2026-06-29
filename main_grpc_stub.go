//go:build small

package main

// initLotusGrpc 小型构建: gRPC 不可用，始终返回 false
func initLotusGrpc(lotus bool, lotusGrpc bool, lotusGrpcPort int) bool {
	return false
}
