package handlers

import (
	"strings"
	"testing"
)

func TestResolveMarkdownMediaReferences(t *testing.T) {
	content := "前置\n![img](file://C:/tmp/a.png)\n[link](file://C:/tmp/b.png)\n"

	replaced := resolveMarkdownMediaReferences(content, func(path string) (string, bool) {
		name := path
		if idx := strings.LastIndex(name, "/"); idx >= 0 {
			name = name[idx+1:]
		}
		if idx := strings.LastIndex(name, "\\"); idx >= 0 {
			name = name[idx+1:]
		}
		return "https://cdn.example.com/" + name, true
	})

	if !strings.Contains(replaced, "https://cdn.example.com/a.png") {
		t.Fatalf("expected image markdown to be rewritten, got: %s", replaced)
	}
	if !strings.Contains(replaced, "https://cdn.example.com/b.png") {
		t.Fatalf("expected markdown link to be rewritten, got: %s", replaced)
	}
	if strings.Contains(replaced, "file://") {
		t.Fatalf("expected local file references to be removed, got: %s", replaced)
	}
}
