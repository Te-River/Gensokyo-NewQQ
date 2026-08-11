package echo

import (
	"sync"
	"testing"
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
