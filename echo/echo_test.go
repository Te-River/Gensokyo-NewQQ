package echo

import (
	"sync"
	"testing"

	"github.com/tencent-connect/botgo/dto"
)

func TestNextMappingSeqConcurrent(t *testing.T) {
	key := "test-next-mapping-seq-concurrent"
	const workers = 128

	values := make(chan int, workers)
	var wg sync.WaitGroup
	wg.Add(workers)
	for i := 0; i < workers; i++ {
		go func() {
			defer wg.Done()
			values <- NextMappingSeq(key)
		}()
	}
	wg.Wait()
	close(values)

	seen := make(map[int]struct{}, workers)
	for value := range values {
		if _, ok := seen[value]; ok {
			t.Fatalf("duplicate sequence value: %d", value)
		}
		seen[value] = struct{}{}
	}
	if len(seen) != workers {
		t.Fatalf("got %d unique values, want %d", len(seen), workers)
	}
	if got := GetMappingSeq(key); got != workers {
		t.Fatalf("stored sequence = %d, want %d", got, workers)
	}
}

func TestCurrentAndIncrementMappingSeqPreservesCurrentValue(t *testing.T) {
	key := "test-current-and-increment-mapping-seq"
	if got := CurrentAndIncrementMappingSeq(key); got != 0 {
		t.Fatalf("first current value = %d, want 0", got)
	}
	if got := GetMappingSeq(key); got != 1 {
		t.Fatalf("stored sequence after first advance = %d, want 1", got)
	}
	if got := CurrentAndIncrementMappingSeq(key); got != 1 {
		t.Fatalf("second current value = %d, want 1", got)
	}
	if got := GetMappingSeq(key); got != 2 {
		t.Fatalf("stored sequence after second advance = %d, want 2", got)
	}
}

func TestStoreRefIdxAndGetRefIdx(t *testing.T) {
	const msgID = "ROBOT1.0_test_refidx_msg"
	const refIDX = "REFIDX_abc123def456=="

	// 清理可能残留
	DeleteRefIdx(msgID)
	if got := GetRefIdx(msgID); got != "" {
		t.Fatalf("expected empty before store, got %q", got)
	}

	StoreRefIdx(msgID, refIDX)
	if got := GetRefIdx(msgID); got != refIDX {
		t.Fatalf("GetRefIdx = %q, want %q", got, refIDX)
	}

	// 空值不存储、不 panic
	StoreRefIdx("", refIDX)
	StoreRefIdx(msgID, "")
	if got := GetRefIdx(msgID); got != refIDX {
		t.Fatalf("empty store should not overwrite, got %q", got)
	}

	DeleteRefIdx(msgID)
	if got := GetRefIdx(msgID); got != "" {
		t.Fatalf("expected empty after delete, got %q", got)
	}
}

func TestStoreRefIdxFromScene(t *testing.T) {
	scene := &dto.MessageScene{
		Source: "default",
		Ext: []string{
			"msg_idx=REFIDX_scene123==",
			"auth_token=sometoken",
		},
	}
	StoreRefIdxFromScene("msg-scene-id", scene)
	if got := GetRefIdx("msg-scene-id"); got != "REFIDX_scene123==" {
		t.Fatalf("GetRefIdx from scene = %q, want %q", got, "REFIDX_scene123==")
	}

	// 无 msg_idx 时不做存储
	sceneNoMsgIdx := &dto.MessageScene{Ext: []string{"auth_token=abc"}}
	StoreRefIdxFromScene("msg-no-idx", sceneNoMsgIdx)
	if got := GetRefIdx("msg-no-idx"); got != "" {
		t.Fatalf("expected empty when no msg_idx, got %q", got)
	}

	// nil scene / 空 msgID 不 panic
	StoreRefIdxFromScene("", scene)
	StoreRefIdxFromScene("msg-nil-scene", nil)
}
