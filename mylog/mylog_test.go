package mylog

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestLogFileSplittingAndRotation(t *testing.T) {
	// Setup a temp log path
	tempLogPath, err := os.MkdirTemp("", "mylog_test_*")
	if err != nil {
		t.Fatalf("failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempLogPath)

	adapter := &MyLogAdapter{
		Level:         LogLevelDebug,
		EnableFileLog: true,
		FileLogPath:   tempLogPath,
	}

	// We temporarily mock the file write with a small max size limit to trigger rotation
	// Write some messages
	adapter.Info("Message 1")
	adapter.Warn("Message 2")
	adapter.Error("Message 3")

	// Verify files are created in logs/YYYY-MM-DD/ folders
	todayStr := time.Now().Format("2006-01-02")
	dayDir := filepath.Join(tempLogPath, todayStr)

	infoFile := filepath.Join(dayDir, "info.log")
	warnFile := filepath.Join(dayDir, "warn.log")
	errFile := filepath.Join(dayDir, "error.log")

	// Read content and check
	checkFileContains := func(path, expected string) {
		data, err := os.ReadFile(path)
		if err != nil {
			t.Fatalf("failed to read file %s: %v", path, err)
		}
		if !strings.Contains(string(data), expected) {
			t.Errorf("file %s does not contain %q, content: %s", path, expected, string(data))
		}
	}

	checkFileContains(infoFile, "Message 1")
	checkFileContains(warnFile, "Message 2")
	checkFileContains(errFile, "Message 3")
}

func TestLogCleaningAndCompression(t *testing.T) {
	tempLogPath, err := os.MkdirTemp("", "mylog_test_clean_*")
	if err != nil {
		t.Fatalf("failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempLogPath)

	// Create a mock old log file (e.g. 10 days ago)
	tenDaysAgo := time.Now().AddDate(0, 0, -10)
	oldDir := filepath.Join(tempLogPath, tenDaysAgo.Format("2006-01-02"))
	err = os.MkdirAll(oldDir, 0755)
	if err != nil {
		t.Fatalf("failed to create old dir: %v", err)
	}

	oldLogFile := filepath.Join(oldDir, "info.log")
	err = os.WriteFile(oldLogFile, []byte("old log content"), 0644)
	if err != nil {
		t.Fatalf("failed to write old file: %v", err)
	}

	// Make the file's ModTime be 10 days ago
	err = os.Chtimes(oldLogFile, tenDaysAgo, tenDaysAgo)
	if err != nil {
		t.Fatalf("failed to chtimes: %v", err)
	}

	// Create a mock expired log file (e.g. 40 days ago)
	fortyDaysAgo := time.Now().AddDate(0, 0, -40)
	expiredDir := filepath.Join(tempLogPath, fortyDaysAgo.Format("2006-01-02"))
	err = os.MkdirAll(expiredDir, 0755)
	if err != nil {
		t.Fatalf("failed to create expired dir: %v", err)
	}

	expiredLogFile := filepath.Join(expiredDir, "info.log")
	err = os.WriteFile(expiredLogFile, []byte("expired log content"), 0644)
	if err != nil {
		t.Fatalf("failed to write expired file: %v", err)
	}

	// Perform cleaning
	CleanLogs(tempLogPath, 30) // max age 30 days

	// Check that 40 days ago is deleted
	if _, err := os.Stat(expiredDir); !os.IsNotExist(err) {
		t.Errorf("expired directory %s should have been deleted", expiredDir)
	}

	// Check that 10 days ago is compressed to gzip
	gzFile := filepath.Join(oldDir, "info.log.gz")
	if _, err := os.Stat(gzFile); os.IsNotExist(err) {
		t.Errorf("old log file %s should have been compressed to %s", oldLogFile, gzFile)
	}
	if _, err := os.Stat(oldLogFile); !os.IsNotExist(err) {
		t.Errorf("original file %s should have been removed after compression", oldLogFile)
	}
}
