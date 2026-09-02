// Nature 图床后端已禁用。
//
// 上游实现曾在公开源码中内置对象存储访问凭据。公开仓库中的凭据无法
// 被安全地视为秘密，因此本分支不再使用、解码或分发这些凭据。
package imagehosting

import "errors"

var errNatureImageHostDisabled = errors.New("Nature 图床已禁用：请改用自行配置凭据的 COS 后端")

func tryNature(_ []byte, _ string) (string, error) {
	return "", errNatureImageHostDisabled
}

