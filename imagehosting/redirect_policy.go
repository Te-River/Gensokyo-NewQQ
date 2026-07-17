package imagehosting

import (
	"fmt"
	"net/http"
)

const maxImageHostRedirects = 5

func init() {
	imageHostingHTTPClient.CheckRedirect = func(req *http.Request, via []*http.Request) error {
		if len(via) >= maxImageHostRedirects {
			return fmt.Errorf("图床重定向次数超过 %d 次", maxImageHostRedirects)
		}
		if err := requireHTTPSURL(req.URL.String()); err != nil {
			return fmt.Errorf("拒绝不安全的图床重定向: %w", err)
		}
		return nil
	}
}
