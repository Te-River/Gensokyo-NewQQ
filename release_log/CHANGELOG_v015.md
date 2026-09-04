# Changelog — Release015

> 自 Release014 以来的所有变更。

---

## Added

- 为 QQ CDN 富媒体上传与统一图床上传添加重试机制：上传失败后最多重试 2 次（线性退避 1s/2s）；CDN 仅对超时类错误重试，图床对所有错误重试。

