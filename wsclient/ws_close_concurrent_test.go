package wsclient

// 补充回归网：CI 失败是并发交错（Close 与 SendMessage 同时发生），
// 而 ws_close_race_test.go 只覆盖了「先 Close 再 SendMessage」的串行路径。

import (
	"errors"
	"sync"
	"testing"
)

// TestConcurrentSendAndCloseNeverPanics 在确认至少一次发送已在途后触发 Close，
// 其余 sender 继续发送；修复前 writeCh 被 Close 关闭，此时必然 panic:
// send on closed channel。断言：只允许 nil 或 errWebSocketClosed，且不 panic。
func TestConcurrentSendAndCloseNeverPanics(t *testing.T) {
	client := newTestWSClient(t)

	const senders = 8
	const perSender = 200

	var wg sync.WaitGroup
	var once sync.Once
	start := make(chan struct{})
	firstSent := make(chan struct{})

	for i := 0; i < senders; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			<-start
			for j := 0; j < perSender; j++ {
				err := client.SendMessage(map[string]interface{}{
					"post_type": "meta_event",
					"sender":    id,
					"seq":       j,
				})
				if err != nil {
					if !errors.Is(err, errWebSocketClosed) {
						t.Errorf("SendMessage 返回非预期错误: %v", err)
						return
					}
					continue
				}
				once.Do(func() { close(firstSent) })
			}
		}(i)
	}

	closerDone := make(chan struct{})
	go func() {
		defer close(closerDone)
		<-firstSent // 保证 Close 发生时写路径正在被使用
		_ = client.Close()
		_ = client.Close() // 并发下的幂等性
	}()

	close(start)
	wg.Wait()
	<-closerDone

	if err := client.SendMessage(map[string]interface{}{"post_type": "meta_event"}); !errors.Is(err, errWebSocketClosed) {
		t.Fatalf("全部发送结束后 SendMessage 应返回 errWebSocketClosed, got %v", err)
	}
}
