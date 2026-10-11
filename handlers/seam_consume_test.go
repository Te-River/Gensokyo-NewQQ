package handlers

// 本文件钉死 S4/S6 接缝的"已消费"语义：
//   - arch_mode=new 时 readLocalMedia 经 MediaService.Prepare；
//   - arch_mode=new 时 nextMappingSeq 经注入的 SequenceRepository；
//   - arch_mode=legacy 时两者均回退 legacy（不触碰注入件）。

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	appmedia "github.com/hoshinonyaruko/gensokyo/internal/application/media"
	appstate "github.com/hoshinonyaruko/gensokyo/internal/application/state"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
	"github.com/hoshinonyaruko/gensokyo/template"
)

// seamLoadArchMode 以完整模板为基底注入指定 arch_mode 并加载 config 单例。
func seamLoadArchMode(t *testing.T, mode string) {
	t.Helper()
	yml := strings.Replace(template.ConfigTemplate, "arch_mode : new", "arch_mode : "+mode, 1)
	path := filepath.Join(t.TempDir(), "config.yml")
	if err := os.WriteFile(path, []byte(yml), 0600); err != nil {
		t.Fatalf("写入测试配置失败: %v", err)
	}
	if _, err := config.LoadConfig(path, false); err != nil {
		t.Fatalf("加载测试配置失败: %v", err)
	}
}

// fakeMediaSvc 记录 Prepare 调用次数并返回固定产物。
type fakeMediaSvc struct {
	calls int
	data  []byte
}

func (f *fakeMediaSvc) Prepare(_ context.Context, _ message.MediaSource, _ appmedia.MediaPolicy) (*appmedia.PreparedMedia, error) {
	f.calls++
	return &appmedia.PreparedMedia{Data: f.data, MIME: "image/png", Size: int64(len(f.data))}, nil
}

// fakeSeqRepo 记录 Next 调用次数。
type fakeSeqRepo struct {
	calls int
	next  uint32
}

func (f *fakeSeqRepo) Next(_ context.Context, _ string) (uint32, error) {
	f.calls++
	return f.next, nil
}

// TestReadLocalMediaUsesMediaServiceInNew 验证 S6：new 路径经 MediaService.Prepare。
func TestReadLocalMediaUsesMediaServiceInNew(t *testing.T) {
	seamLoadArchMode(t, "new")
	f := &fakeMediaSvc{data: []byte("prepared")}
	SetMediaService(f, appmedia.MediaPolicy{})
	defer SetMediaService(nil, appmedia.MediaPolicy{})

	got, err := readLocalMedia("whatever.png")
	if err != nil {
		t.Fatalf("readLocalMedia 失败: %v", err)
	}
	if f.calls != 1 {
		t.Fatalf("new 路径应调用 MediaService.Prepare: calls=%d", f.calls)
	}
	if string(got) != "prepared" {
		t.Fatalf("应返回 Prepare 产物: got=%q", got)
	}
}

// TestReadLocalMediaLegacyFallback 验证 S6：legacy 不触碰 MediaService，直读文件。
func TestReadLocalMediaLegacyFallback(t *testing.T) {
	seamLoadArchMode(t, "legacy")
	f := &fakeMediaSvc{data: []byte("prepared")}
	SetMediaService(f, appmedia.MediaPolicy{})
	defer SetMediaService(nil, appmedia.MediaPolicy{})

	path := filepath.Join(t.TempDir(), "a.png")
	if err := os.WriteFile(path, []byte("real"), 0600); err != nil {
		t.Fatalf("写入临时文件失败: %v", err)
	}
	got, err := readLocalMedia(path)
	if err != nil {
		t.Fatalf("readLocalMedia 失败: %v", err)
	}
	if f.calls != 0 {
		t.Fatalf("legacy 不应调用 MediaService: calls=%d", f.calls)
	}
	if string(got) != "real" {
		t.Fatalf("legacy 应读原文件: got=%q", got)
	}
}

// TestNextMappingSeqUsesRepositoryInNew 验证 S4：new 路径经注入的 SequenceRepository。
func TestNextMappingSeqUsesRepositoryInNew(t *testing.T) {
	seamLoadArchMode(t, "new")
	f := &fakeSeqRepo{next: 42}
	SetStateRepositories(f, nil)
	defer SetStateRepositories(nil, nil)

	if got := nextMappingSeq("k"); got != 42 {
		t.Fatalf("new 路径应经仓储返回: got=%d", got)
	}
	if f.calls != 1 {
		t.Fatalf("new 路径应调用 SequenceRepository.Next: calls=%d", f.calls)
	}
}

// TestNextMappingSeqLegacyDirect 验证 S4：legacy 直调 echo，不触碰仓储。
func TestNextMappingSeqLegacyDirect(t *testing.T) {
	seamLoadArchMode(t, "legacy")
	f := &fakeSeqRepo{next: 42}
	SetStateRepositories(f, nil)
	defer SetStateRepositories(nil, nil)

	got := nextMappingSeq("seam-legacy-key")
	if f.calls != 0 {
		t.Fatalf("legacy 不应调用仓储: calls=%d", f.calls)
	}
	// 两个 fresh key 直调 echo 均返回 1；证明 legacy 路径仍走 echo。
	if want := echo.IncrementMappingSeq("seam-legacy-key-2"); got != want {
		t.Fatalf("legacy 应与 echo.IncrementMappingSeq 一致: got=%d want=%d", got, want)
	}
}

var _ appstate.SequenceRepository = (*fakeSeqRepo)(nil)
