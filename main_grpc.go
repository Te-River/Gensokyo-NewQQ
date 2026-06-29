//go:build !small

package main

import (
	"fmt"
	"log"
	"net"
	"strconv"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	proto "github.com/hoshinonyaruko/gensokyo/proto"
	"google.golang.org/grpc"
)

// initLotusGrpc 初始化 gRPC 客户端或服务端（Lotus 模式）
// 返回 true 表示已使用 gRPC（路由 /getid 不应注册 HTTP handler）
func initLotusGrpc(lotus bool, lotusGrpc bool, lotusGrpcPort int) bool {
	if !lotusGrpc {
		return false
	}

	if lotus {
		// Lotus 子进程模式：初始化 gRPC 客户端
		if config.GetLotusGrpc() {
			serverDir := config.GetServer_dir()
			conn, err := grpc.NewClient(serverDir+":"+strconv.Itoa(lotusGrpcPort), grpc.WithInsecure())
			if err != nil {
				panic(fmt.Sprintf("failed to connect to gRPC server: %v", err))
			} else {
				fmt.Printf("成功连接到GRPC服务器: %v\n", serverDir+":50051")
			}
			idmap.GrpcClient = proto.NewIDMapServiceClient(conn)
		}
	} else {
		// Lotus 主进程模式：初始化 gRPC 服务端
		lis, err := net.Listen("tcp", ":"+strconv.Itoa(lotusGrpcPort))
		if err != nil {
			log.Fatalf("failed to listen: %v", err)
		}

		grpcServer := grpc.NewServer()
		proto.RegisterIDMapServiceServer(grpcServer, &idmap.Server{})

		log.Println("Starting gRPC server on port :" + strconv.Itoa(lotusGrpcPort))
		if err := grpcServer.Serve(lis); err != nil {
			log.Fatalf("failed to serve: %v", err)
		}
	}
	return true
}
