package Processor

import (
	"bytes"
	"fmt"
	"net/http"
	"sync"

	"github.com/hashicorp/go-multierror"
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/handlers"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func (p *Processors) SendMessageToAllClients(message map[string]interface{}) error {
	applyOpUserIDType(message)
	var result *multierror.Error

	for _, client := range p.WsServerClients {
		err := client.SendMessage(message)
		if err != nil {
			result = multierror.Append(result, fmt.Errorf("failed to send to client: %w", err))
		}
	}

	return result.ErrorOrNil()
}

func (p *Processors) BroadcastMessageToAllFAF(message map[string]interface{}, api openapi.MessageAPI, data interface{}) error {
	applyOpUserIDType(message)
	for _, client := range p.Wsclient {
		go func(c callapi.WebSocketServerClienter) {
			_ = c.SendMessage(message)
		}(client)
	}

	for _, serverClient := range p.WsServerClients {
		go func(sc callapi.WebSocketServerClienter) {
			_ = sc.SendMessage(message)
		}(serverClient)
	}

	return nil
}

func (p *Processors) BroadcastMessageToAll(message map[string]interface{}, api openapi.MessageAPI, data interface{}) error {
	applyOpUserIDType(message)
	var wg sync.WaitGroup
	errorCh := make(chan string, len(p.Wsclient)+len(p.WsServerClients))
	defer close(errorCh)

	for _, client := range p.Wsclient {
		wg.Add(1)
		go func(c callapi.WebSocketServerClienter) {
			defer wg.Done()
			if err := c.SendMessage(message); err != nil {
				errorCh <- fmt.Sprintf("Error sending to wsclient: %v", err)
			}
		}(client)
	}

	for _, serverClient := range p.WsServerClients {
		wg.Add(1)
		go func(sc callapi.WebSocketServerClienter) {
			defer wg.Done()
			if err := sc.SendMessage(message); err != nil {
				errorCh <- fmt.Sprintf("Error sending to server client: %v", err)
			}
		}(serverClient)
	}

	wg.Wait()

	select {
	case errMsg := <-errorCh:
		mylog.Printf("BroadcastMessageToAll error: %s", errMsg)
	default:
	}

	return nil
}

func allEmpty(addresses []string) bool {
	for _, addr := range addresses {
		if addr != "" {
			return false
		}
	}
	return true
}

func PostMessageToUrls(message map[string]interface{}) {
	postUrls := config.GetPostUrl()

	if len(postUrls) == 0 {
		return
	}

	jsonString, err := handlers.ConvertMapToJSONString(message)
	if err != nil {
		mylog.Printf("Error converting message to JSON: %v", err)
		return
	}

	var wg sync.WaitGroup
	for _, url := range postUrls {
		wg.Add(1)
		go func(url string) {
			defer wg.Done()
			sendPostRequest(jsonString, url)
		}(url)
	}
	wg.Wait()
}

func sendPostRequest(jsonString, url string) {
	reqBody := bytes.NewBufferString(jsonString)

	req, err := http.NewRequest("POST", url, reqBody)
	if err != nil {
		mylog.Printf("Error creating POST request to %s: %v", url, err)
		return
	}

	req.Header.Set("Content-Type", "application/json")
	var selfid string
	if config.GetUseUin() {
		selfid = config.GetUinStr()
	} else {
		selfid = config.GetAppIDStr()
	}
	req.Header.Set("X-Self-ID", selfid)

	client := &http.Client{}
	resp, err := client.Do(req)
	if err != nil {
		mylog.Printf("Error sending POST request to %s: %v", url, err)
		return
	}
	defer resp.Body.Close()

	mylog.Printf("Posted to %s successfully", url)
}