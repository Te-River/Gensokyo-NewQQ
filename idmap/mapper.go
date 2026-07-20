package idmap

// IDMapper 定义了 ID 映射的核心接口
// 统一新旧两套 idmap 系统的操作契约
type IDMapper interface {
	// StoreID 存储 ID 并返回虚拟 ID
	StoreID(id string) (int64, error)

	// StoreIDv2 存储 ID v2 版本
	StoreIDv2(id string) (int64, error)

	// StoreCache 存储缓存 ID
	StoreCache(id string) (int64, error)

	// StoreCachev2 存储缓存 ID v2
	StoreCachev2(id string) (int64, error)

	// RetrieveRowByID 根据虚拟 ID 查询真实 ID
	RetrieveRowByID(rowid string) (string, error)

	// RetrieveRowByIDv2 根据虚拟 ID 查询真实 ID v2
	RetrieveRowByIDv2(rowid string) (string, error)

	// RetrieveRowByCache 根据缓存虚拟 ID 查询真实 ID
	RetrieveRowByCache(rowid string) (string, error)

	// RetrieveRowByCachev2 根据缓存虚拟 ID 查询真实 ID v2
	RetrieveRowByCachev2(rowid string) (string, error)

	// GenerateRowID 生成行 ID
	GenerateRowID(id string, length int) (int64, error)

	// Close 关闭数据库连接
	CloseDB()
}

// NewIDMapper 创建新的 IDMapper 实例（默认使用新系统）
func NewIDMapper() IDMapper {
	initNewDBs()
	return &newMapper{}
}

// newMapper 实现 IDMapper 接口，使用新 idmap 系统
type newMapper struct{}

func (m *newMapper) StoreID(id string) (int64, error)    { return StoreID(id) }
func (m *newMapper) StoreIDv2(id string) (int64, error)  { return StoreIDv2(id) }
func (m *newMapper) StoreCache(id string) (int64, error) { return StoreCache(id) }
func (m *newMapper) StoreCachev2(id string) (int64, error) { return StoreCachev2(id) }
func (m *newMapper) RetrieveRowByID(rowid string) (string, error)   { return RetrieveRowByID(rowid) }
func (m *newMapper) RetrieveRowByIDv2(rowid string) (string, error) { return RetrieveRowByIDv2(rowid) }
func (m *newMapper) RetrieveRowByCache(rowid string) (string, error) { return RetrieveRowByCache(rowid) }
func (m *newMapper) RetrieveRowByCachev2(rowid string) (string, error) { return RetrieveRowByCachev2(rowid) }
func (m *newMapper) GenerateRowID(id string, length int) (int64, error) { return GenerateRowID(id, length) }
func (m *newMapper) CloseDB() { CloseDB() }