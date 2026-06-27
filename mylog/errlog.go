package mylog

import (
	"encoding/json"
	"fmt"
	"log"
)

// 独立的错误日志记录函数
func ErrLogToFile(level, message string) {
	if !enableFileLogGlobal {
		return
	}
	LogToFile("ERROR", fmt.Sprintf("[%s] %s", level, message))
}

// 独立的错误日志记录函数
func ErrInterfaceToFile(level, message interface{}) {
	if !enableFileLogGlobal {
		return
	}

	jsonData, err := json.Marshal(message)
	if err != nil {
		log.Printf("Error marshaling data for log: %s", err)
		return
	}

	LogToFile("ERROR", fmt.Sprintf("[%s] %s", level, string(jsonData)))
}
