package messagequeue

import (
	"sync"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/mylog"
)

// MessageTask 消息任务
type MessageTask struct {
	ID        string
	Type      string // "group", "private", "guild_channel"
	TargetID  string
	Content   interface{}
	CreatedAt time.Time
	RetryCount int
	MaxRetries int
}

// Queue 消息队列
type Queue struct {
	mu       sync.Mutex
	tasks    []*MessageTask
	cond     *sync.Cond
	closed   bool
	wg       sync.WaitGroup
}

var (
	instance *Queue
	once     sync.Once
)

// GetQueue 获取全局消息队列实例
func GetQueue() *Queue {
	once.Do(func() {
		instance = &Queue{
			tasks: make([]*MessageTask, 0, 100),
		}
		instance.cond = sync.NewCond(&instance.mu)
	})
	return instance
}

// Enqueue 添加消息到队列
func (q *Queue) Enqueue(task *MessageTask) {
	q.mu.Lock()
	defer q.mu.Unlock()
	if q.closed {
		return
	}
	task.CreatedAt = time.Now()
	task.MaxRetries = 3
	q.tasks = append(q.tasks, task)
	q.cond.Signal()
}

// Dequeue 从队列取消息（阻塞）
func (q *Queue) Dequeue() *MessageTask {
	q.mu.Lock()
	defer q.mu.Unlock()
	for len(q.tasks) == 0 && !q.closed {
		q.cond.Wait()
	}
	if q.closed {
		return nil
	}
	task := q.tasks[0]
	q.tasks = q.tasks[1:]
	return task
}

// Len 当前队列长度
func (q *Queue) Len() int {
	q.mu.Lock()
	defer q.mu.Unlock()
	return len(q.tasks)
}

// Close 关闭队列
func (q *Queue) Close() {
	q.mu.Lock()
	defer q.mu.Unlock()
	q.closed = true
	q.cond.Broadcast()
}

// Wait 等待所有任务处理完毕
func (q *Queue) Wait() {
	q.wg.Wait()
}

// StartWorker 启动工作协程
func StartWorker(workerID int, processFn func(*MessageTask) bool) {
	q := GetQueue()
	delay := time.Duration(config.GetSendDelay()) * time.Millisecond
	if delay <= 0 {
		delay = 300 * time.Millisecond
	}

	go func() {
		mylog.Printf("[消息队列] 工作协程 #%d 已启动 (发送间隔: %v)", workerID, delay)
		for {
			task := q.Dequeue()
			if task == nil {
				return
			}

			// 发送间隔
			time.Sleep(delay)

			success := processFn(task)
			if !success && task.RetryCount < task.MaxRetries {
				task.RetryCount++
				mylog.Printf("[消息队列] 消息 %s 发送失败，第 %d 次重试", task.ID, task.RetryCount)
				time.Sleep(time.Second * time.Duration(task.RetryCount))
				q.Enqueue(task)
			}
		}
	}()
}
