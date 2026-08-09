package server

import (
  "crypto/hmac"
  "crypto/sha256"
  "encoding/hex"
  "net/http"
  "sort"
  "strconv"
  "strings"
  "time"

  "github.com/gin-gonic/gin"
  "github.com/hoshinonyaruko/gensokyo/config"
  "github.com/hoshinonyaruko/gensokyo/idmap"
  "github.com/hoshinonyaruko/gensokyo/mylog"
)

// IDMapAuthMiddleware 为 /getid 端点提供认证保护
// 使用 HMAC-SHA256(password, path + "?" + sortedQueryParams + "&timestamp=" + timestamp) 进行认证
// 服务端校验时间窗口为 ±5 分钟，防止重放攻击
func IDMapAuthMiddleware() gin.HandlerFunc {
	return func(c *gin.Context) {
		password := config.GetLotusPassword()
		if password != "" {
			// 有 Lotus password 时，使用 HMAC-SHA256 token 认证
			token := c.Query("token")
			if token == "" {
				token = c.GetHeader("X-Token")
			}
			if token == "" {
				c.JSON(http.StatusUnauthorized, gin.H{"error": "missing token"})
				c.Abort()
				return
			}

			// 从 query 中获取时间戳并校验时间窗口
			timestampStr := c.Query("timestamp")
			if timestampStr == "" {
				c.JSON(http.StatusUnauthorized, gin.H{"error": "missing timestamp"})
				c.Abort()
				return
			}
			timestamp, err := strconv.ParseInt(timestampStr, 10, 64)
			if err != nil {
				c.JSON(http.StatusUnauthorized, gin.H{"error": "invalid timestamp"})
				c.Abort()
				return
			}
			now := time.Now().Unix()
			if now-timestamp > 300 || timestamp-now > 300 {
				c.JSON(http.StatusUnauthorized, gin.H{"error": "timestamp expired"})
				c.Abort()
				return
			}

			// 重建签名字符串：path + "?" + sorted query params (不含 token 和 timestamp) + "&timestamp=" + timestampStr
			queryParams := c.Request.URL.Query()
			// 移除 token 和 timestamp 参数，它们不参与签名
			delete(queryParams, "token")
			delete(queryParams, "timestamp")

			// 对剩余参数按键排序，保证签名一致性
			paramKeys := make([]string, 0, len(queryParams))
			for k := range queryParams {
				paramKeys = append(paramKeys, k)
			}
			sort.Strings(paramKeys)

			var paramParts []string
			for _, k := range paramKeys {
				for _, v := range queryParams[k] {
					paramParts = append(paramParts, k+"="+v)
				}
			}

			signPayload := c.Request.URL.Path
			if len(paramParts) > 0 {
				signPayload += "?" + strings.Join(paramParts, "&")
			}
			signPayload += "&timestamp=" + timestampStr

			mac := hmac.New(sha256.New, []byte(password))
			mac.Write([]byte(signPayload))
			expected := hex.EncodeToString(mac.Sum(nil))
			if token != expected {
				c.JSON(http.StatusUnauthorized, gin.H{"error": "unauthorized"})
				c.Abort()
				return
			}
		} else {
			// 无密码时，仅允许本地回环地址访问
			remoteIP := c.ClientIP()
			if remoteIP != "127.0.0.1" && remoteIP != "::1" && remoteIP != "localhost" {
				c.JSON(http.StatusForbidden, gin.H{"error": "仅允许本地访问"})
				c.Abort()
				return
			}
		}
		c.Next()
	}
}

func GetIDHandler(c *gin.Context) {
	idOrRow := c.Query("id")
	typeVal, err := strconv.Atoi(c.Query("type"))

	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "invalid type"})
		return
	}

	switch typeVal {
	case 1:
		newRow, err := idmap.StoreID(idOrRow)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"row": newRow})

	case 2:
		id, err := idmap.RetrieveRowByID(idOrRow)
		if err == idmap.ErrKeyNotFound {
			c.JSON(http.StatusNotFound, gin.H{"error": "ID not found"})
			return
		} else if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"id": id})

	case 3:
		// 存储
		section := c.Query("id")
		subtype := c.Query("subtype")
		value := c.Query("value")
		err := idmap.WriteConfig(section, subtype, value)
		if err != nil {
			mylog.Printf("%s", err.Error())
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"status": "success"})

	case 4:
		// 获取值
		section := c.Query("id")
		subtype := c.Query("subtype")
		value, err := idmap.ReadConfig(section, subtype)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"value": value})

	case 5:
		oldRowValue, err := strconv.ParseInt(c.Query("oldRowValue"), 10, 64)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "invalid oldRowValue"})
			return
		}

		newRowValue, err := strconv.ParseInt(c.Query("newRowValue"), 10, 64)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "invalid newRowValue"})
			return
		}

		err = idmap.UpdateVirtualValue(oldRowValue, newRowValue)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"status": "success"})

	case 6:
		virtualValue, err := strconv.ParseInt(c.Query("virtualValue"), 10, 64)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "invalid virtualValue"})
			return
		}

		virtual, real, err := idmap.RetrieveRealValue(virtualValue)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"virtual": virtual, "real": real})
	case 7:
		realValue := c.Query("id")
		if realValue == "" {
			c.JSON(http.StatusBadRequest, gin.H{"error": "invalid id"})
			return
		}

		_, virtualValue, err := idmap.RetrieveVirtualValue(realValue)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"virtual": virtualValue})
	case 8:
		// 调用新的 StoreIDv2Pro
		subid := c.Query("subid")
		if subid == "" {
			c.JSON(http.StatusBadRequest, gin.H{"error": "subid parameter is required for type 8"})
			return
		}
		newRow, newSubRow, err := idmap.StoreIDv2Pro(idOrRow, subid)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"row": newRow, "subRow": newSubRow})

	case 9:
		// 调用新的 RetrieveRowByIDv2Pro
		subid := c.Query("subid")
		if subid == "" {
			c.JSON(http.StatusBadRequest, gin.H{"error": "subid parameter is required for type 9"})
			return
		}
		id, subid, err := idmap.RetrieveRowByIDv2Pro(idOrRow, subid)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"id": id, "subid": subid})
	case 10:
		subid := c.Query("subid")
		if idOrRow == "" || subid == "" {
			c.JSON(http.StatusBadRequest, gin.H{"error": "id and subid parameters are required for type 10"})
			return
		}

		firstValue, secondValue, err := idmap.RetrieveVirtualValuev2Pro(idOrRow, subid) // 确保函数名称正确
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}

		c.JSON(http.StatusOK, gin.H{"firstValue": firstValue, "secondValue": secondValue})
	case 11:
		subid := c.Query("subid")
		if idOrRow == "" || subid == "" {
			c.JSON(http.StatusBadRequest, gin.H{"error": "id and subid parameters are required for type 11"})
			return
		}
		var virtualValue, virtualValueSub int64
		virtualValue, err = strconv.ParseInt(idOrRow, 10, 64)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid input values idOrRow"})
			return
		}
		virtualValueSub, err = strconv.ParseInt(subid, 10, 64)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid input values subid"})
			return
		}
		firstRealValue, secondRealValue, err := idmap.RetrieveRealValuesv2Pro(virtualValue, virtualValueSub)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}

		c.JSON(http.StatusOK, gin.H{"firstRealValue": firstRealValue, "secondRealValue": secondRealValue})
	case 12:
		oldVirtualValue1Str := c.Query("oldVirtualValue1")
		newVirtualValue1Str := c.Query("newVirtualValue1")
		oldVirtualValue2Str := c.Query("oldVirtualValue2")
		newVirtualValue2Str := c.Query("newVirtualValue2")
		var oldVirtualValue1, newVirtualValue1, oldVirtualValue2, newVirtualValue2 int64
		// 将字符串转换为int64
		oldVirtualValue1, err = strconv.ParseInt(oldVirtualValue1Str, 10, 64)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid input values oldVirtualValue1"})
			return
		}
		newVirtualValue1, err = strconv.ParseInt(newVirtualValue1Str, 10, 64)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid input values newVirtualValue1"})
			return
		}
		oldVirtualValue2, err = strconv.ParseInt(oldVirtualValue2Str, 10, 64)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid input values oldVirtualValue2"})
			return
		}
		newVirtualValue2, err = strconv.ParseInt(newVirtualValue2Str, 10, 64)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid input values newVirtualValue2"})
			return
		}
		err = idmap.UpdateVirtualValuev2Pro(oldVirtualValue1, newVirtualValue1, oldVirtualValue2, newVirtualValue2)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}

		c.JSON(http.StatusOK, gin.H{"message": "Virtual values updated successfully"})
	case 13:
		newRow, err := idmap.SimplifiedStoreIDv2(idOrRow)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"row": newRow})
	case 14:
		id := c.Query("id")
		keys, err := idmap.FindSubKeysByIdPro(id)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"keys": keys})
	case 15:
		// 删除
		// 从请求中获取参数
		section := c.Query("id")
		subtype := c.Query("subtype")

		// 调用DeleteConfigv2来删除配置
		err := idmap.DeleteConfigv2(section, subtype)
		if err != nil {
			// 如果有错误，记录并返回错误信息
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}

		// 如果删除成功，返回成功响应
		c.JSON(http.StatusOK, gin.H{"status": "success"})
	case 16:
		newRow, err := idmap.StoreCachev2(idOrRow)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"row": newRow})

	case 17:
		id, err := idmap.RetrieveRowByCachev2(idOrRow)
		if err == idmap.ErrKeyNotFound {
			c.JSON(http.StatusNotFound, gin.H{"error": "ID not found"})
			return
		} else if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"id": id})
	case 18:
		// 强制解绑指定 OpenID 的所有映射
		// 用途：迁移期间双写导致 2 个虚拟 ID 指向同一 OpenID 时，按 OpenID 直接清理全部映射
		openID := c.Query("id")
		if openID == "" {
			c.JSON(http.StatusBadRequest, gin.H{"error": "id parameter is required for type 18"})
			return
		}
		unboundCount, err := idmap.ForceUnbindID(openID)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, gin.H{"status": "success", "unbound_count": unboundCount})
	case 19:
		// 批量导出所有身份映射（OpenID <-> 虚拟 ID）
		// 用途：迁移调试或数据核查时，一次性导出所有身份映射
		snapshot, err := idmap.ListAllIdentities()
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		c.JSON(http.StatusOK, snapshot)
	}

}
