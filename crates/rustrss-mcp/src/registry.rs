//! 工具注册表：每个工具的 `scope(read|write)` 与 `dangerous(bool)` 元数据。
//!
//! **一张表管两件事**：`tools/list` 的可见性过滤与工具调用的授权闸门（gating）
//! 都只查这里——两处如果各写一份判断，迟早会出现「列出来了但调不动」或
//! 「没列出来却能调」的漂移。`visible()` 就是 `authorize().is_ok()`，不是近似。
//!
//! 授权模型（PRD「授权模型」/ tech_design 同口径）：
//! - **认证 vs 授权分离**：token 对不对是传输层的事（错 → 401）；
//!   「这个请求能不能写」在这里裁决，返回**工具级错误码**（200 + isError）。
//! - **凭据 → scope**：HTTP 由当次请求携带的凭据现算（写 token → `write`，
//!   读 token → `read`）；stdio 没有凭据概念，恒为 `write`，写能力由下面的开关把关。
//! - **三道闸**（顺序固定，先说的先判）：
//!   ① 写工具需要 `write` scope（读 token 调用 → `write_scope_required`）；
//!   ② 写能力总开关 `mcp.write_enabled` 必须开（→ `write_disabled`）；
//!   ③ 写 token 必须已生成（→ `write_disabled`，与 ② 同一个码，消息里点明缺哪一环）；
//!   ④ 危险工具还需要 `mcp.dangerous_enabled`（→ `dangerous_tool_disabled`）。
//!
//! 三个开关默认全关（键缺失即关）：装好就是只读，写能力要靠用户在设置页显式开。

use rmcp::model::Tool;

/// 工具作用域
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// 只读工具：任何已认证的请求都能调
    Read,
    /// 写工具：需要写凭据（HTTP）或写开关（stdio），且写 token 已生成
    Write,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Write => "write",
        }
    }
}

/// 一个工具的授权元数据
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: &'static str,
    pub scope: Scope,
    /// 危险工具：破坏性/不可逆（退订级联删条目、删分组），需要 `confirm` 与单独开关
    pub dangerous: bool,
}

/// 授权决策需要的开关快照（每次请求现读，不缓存）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Switches {
    /// `mcp.write_enabled`（默认 false）
    pub write_enabled: bool,
    /// `mcp.dangerous_enabled`（默认 false）
    pub dangerous_enabled: bool,
    /// `mcp.write_token` 是否存在且非空（不存在 = 写能力没被显式provision）
    pub write_token: bool,
}

/// 未获授权的写调用：`code()` 是给 agent 看的机器可读错误码（PRD 错误码表内）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateError {
    /// 该请求没有写凭据（读 token / 未带写 token）
    WriteScopeRequired,
    /// 写能力总开关关着
    WriteSwitchOff,
    /// 写 token 未生成（写能力没被显式开启）
    WriteTokenMissing,
    /// 危险工具的开关关着
    DangerousToolDisabled,
}

impl GateError {
    pub fn code(self) -> &'static str {
        match self {
            GateError::WriteScopeRequired => "write_scope_required",
            GateError::WriteSwitchOff | GateError::WriteTokenMissing => "write_disabled",
            GateError::DangerousToolDisabled => "dangerous_tool_disabled",
        }
    }

    /// 给 agent 的说明：不会泄露数据，且点明「怎么才能拿到权限」
    pub fn message(self) -> &'static str {
        match self {
            GateError::WriteScopeRequired => {
                "该请求没有写权限：写工具只对携带写 token 的连接开放（stdio 由应用的写开关决定）"
            }
            GateError::WriteSwitchOff => {
                "写能力未开启：请在 设置 → MCP 打开「写能力」开关（默认关闭）"
            }
            GateError::WriteTokenMissing => {
                "写能力未开启：请在 设置 → MCP 生成写 token（未生成时写工具不注册）"
            }
            GateError::DangerousToolDisabled => {
                "危险工具未开启：请在 设置 → MCP 打开「危险工具」开关后再调用"
            }
        }
    }

    /// 工具级错误信封（与写操作返回结构同形：`{ ok, affected, results, error_code }`）
    pub fn to_body(self) -> String {
        serde_json::json!({
            "ok": false,
            "affected": 0,
            "results": [],
            "error_code": self.code(),
            "error": self.message(),
        })
        .to_string()
    }
}

/// 已登记的工具清单。
///
/// 新增工具必须同时在这里登记——`lib.rs` 的测试
/// `every_exposed_tool_is_registered` 会把「`#[tool]` 里有、注册表里没有」
/// 判成失败，所以漏登记不会悄悄溜过去（漏登记 = 默认按无权限处理）。
pub const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec {
        name: "list_feeds",
        scope: Scope::Read,
        dangerous: false,
    },
    ToolSpec {
        name: "list_folders",
        scope: Scope::Read,
        dangerous: false,
    },
    ToolSpec {
        name: "list_articles",
        scope: Scope::Read,
        dangerous: false,
    },
    ToolSpec {
        name: "get_article",
        scope: Scope::Read,
        dangerous: false,
    },
    ToolSpec {
        name: "search_articles",
        scope: Scope::Read,
        dangerous: false,
    },
    ToolSpec {
        name: "get_unread_summary",
        scope: Scope::Read,
        dangerous: false,
    },
    ToolSpec {
        name: "db_stats",
        scope: Scope::Read,
        dangerous: false,
    },
    // T3：阅读状态与刷新。三者都不危险（可逆/幂等/不删数据），所以不受危险开关约束，
    // 但仍需要写 scope + 写开关 + 写 token 三道闸。
    ToolSpec {
        name: "set_read",
        scope: Scope::Write,
        dangerous: false,
    },
    ToolSpec {
        name: "set_starred",
        scope: Scope::Write,
        dangerous: false,
    },
    ToolSpec {
        name: "set_read_later",
        scope: Scope::Write,
        dangerous: false,
    },
    ToolSpec {
        name: "refresh",
        scope: Scope::Write,
        dangerous: false,
    },
    ToolSpec {
        name: "fetch_fulltext",
        scope: Scope::Write,
        dangerous: false,
    },
    // T4：订阅管理。危险工具（`unsubscribe` / `folder_delete`）**只多一道危险开关**，
    // 其余闸门与普通写工具完全一样（写 scope → 写开关 → 写 token → 危险开关）。
    ToolSpec {
        name: "subscribe",
        scope: Scope::Write,
        dangerous: false,
    },
    ToolSpec {
        name: "update_feed",
        scope: Scope::Write,
        dangerous: false,
    },
    ToolSpec {
        name: "folder_create",
        scope: Scope::Write,
        dangerous: false,
    },
    ToolSpec {
        name: "folder_rename",
        scope: Scope::Write,
        dangerous: false,
    },
    ToolSpec {
        name: "folder_delete",
        scope: Scope::Write,
        dangerous: true,
    },
    ToolSpec {
        name: "unsubscribe",
        scope: Scope::Write,
        dangerous: true,
    },
    ToolSpec {
        name: "import_opml",
        scope: Scope::Write,
        dangerous: false,
    },
    ToolSpec {
        name: "export_opml",
        scope: Scope::Write,
        dangerous: false,
    },
];

/// 已登记工具的只读子集数量（读 token 会话 `tools/list` 的期待值）
pub fn read_tool_count() -> usize {
    TOOL_SPECS.iter().filter(|s| s.scope == Scope::Read).count()
}

/// 静态注册表查询
pub fn spec(name: &str) -> Option<&'static ToolSpec> {
    TOOL_SPECS.iter().find(|s| s.name == name)
}

/// 工具调用的授权裁决（写工具才需要过闸；读工具直接放行）。
///
/// 顺序：scope → 写开关 → 写 token 存在性 → 危险开关。
/// 未登记的工具名**不在这里报错**：那是「工具不存在」，交给 router 按 MCP 语义拒绝。
pub fn authorize(spec: &ToolSpec, scope: Scope, switches: &Switches) -> Result<(), GateError> {
    if spec.scope != Scope::Write {
        return Ok(());
    }
    if scope != Scope::Write {
        return Err(GateError::WriteScopeRequired);
    }
    if !switches.write_enabled {
        return Err(GateError::WriteSwitchOff);
    }
    if !switches.write_token {
        return Err(GateError::WriteTokenMissing);
    }
    if spec.dangerous && !switches.dangerous_enabled {
        return Err(GateError::DangerousToolDisabled);
    }
    Ok(())
}

/// 可见性 = 可调用性：`tools/list` 过滤直接用 `authorize`，
/// 「列出来却调不动」与「藏起来却能调」两种不一致都在这条式子上被排除。
pub fn visible(spec: &ToolSpec, scope: Scope, switches: &Switches) -> bool {
    authorize(spec, scope, switches).is_ok()
}

/// 过滤工具列表（未知工具名保守处理：**过滤掉**，宁可少列也不越权）
pub fn filter_visible(tools: Vec<Tool>, scope: Scope, switches: &Switches) -> Vec<Tool> {
    tools
        .into_iter()
        .filter(|t| match spec(&t.name) {
            Some(s) => visible(s, scope, switches),
            None => false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_spec(dangerous: bool) -> ToolSpec {
        ToolSpec {
            name: "stub_write",
            scope: Scope::Write,
            dangerous,
        }
    }

    fn read_spec() -> ToolSpec {
        ToolSpec {
            name: "stub_read",
            scope: Scope::Read,
            dangerous: false,
        }
    }

    fn all_on() -> Switches {
        Switches {
            write_enabled: true,
            dangerous_enabled: true,
            write_token: true,
        }
    }

    /// 读工具不受任何开关影响（否则一开写能力就会把只读工具锁住）
    #[test]
    fn read_tools_ignore_switches_and_scope() {
        let off = Switches::default();
        for scope in [Scope::Read, Scope::Write] {
            assert!(authorize(&read_spec(), scope, &off).is_ok());
            assert!(visible(&read_spec(), scope, &off));
        }
    }

    /// 授权矩阵：读 token / 总开关 / 写 token 存在性 / 危险开关，逐格断言错误码
    #[test]
    fn write_tool_authorization_matrix() {
        // ① 读 scope（HTTP 读 token）：先于开关判定 → write_scope_required
        for switches in [Switches::default(), all_on()] {
            let err = authorize(&write_spec(false), Scope::Read, &switches).unwrap_err();
            assert_eq!(err.code(), "write_scope_required");
        }

        // ② 写 scope 但总开关关着 → write_disabled
        let err = authorize(
            &write_spec(false),
            Scope::Write,
            &Switches {
                write_enabled: false,
                ..all_on()
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "write_disabled");
        assert!(err.message().contains("写能力"), "{}", err.message());

        // ③ 写 scope、总开关开、但写 token 未生成 → write_disabled（消息点明缺 token）
        let err = authorize(
            &write_spec(false),
            Scope::Write,
            &Switches {
                write_token: false,
                ..all_on()
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "write_disabled");
        assert!(err.message().contains("写 token"), "{}", err.message());

        // ④ 危险工具：前两道过了，危险开关关着 → dangerous_tool_disabled
        let err = authorize(
            &write_spec(true),
            Scope::Write,
            &Switches {
                dangerous_enabled: false,
                ..all_on()
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "dangerous_tool_disabled");

        // ⑤ 全开 → 写工具与危险写工具都放行
        assert!(authorize(&write_spec(false), Scope::Write, &all_on()).is_ok());
        assert!(authorize(&write_spec(true), Scope::Write, &all_on()).is_ok());

        // 危险开关只影响危险工具：普通写工具不看它
        assert!(authorize(
            &write_spec(false),
            Scope::Write,
            &Switches {
                dangerous_enabled: false,
                ..all_on()
            }
        )
        .is_ok());
    }

    /// 可见性必须与 gating 完全一致（同一函数）：不出现「列出来却调不动」
    #[test]
    fn visibility_matches_authorization_cell_by_cell() {
        for scope in [Scope::Read, Scope::Write] {
            for write_enabled in [false, true] {
                for write_token in [false, true] {
                    for dangerous_enabled in [false, true] {
                        let sw = Switches {
                            write_enabled,
                            dangerous_enabled,
                            write_token,
                        };
                        for spec in [read_spec(), write_spec(false), write_spec(true)] {
                            assert_eq!(
                                visible(&spec, scope, &sw),
                                authorize(&spec, scope, &sw).is_ok(),
                                "{spec:?} {scope:?} {sw:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// 未知工具名不越权：过滤时直接丢掉（默认不可见）
    #[test]
    fn unknown_tools_are_filtered_out() {
        let tool = |name: &'static str| Tool::new(name, "", std::sync::Arc::new(serde_json::Map::new()));
        let kept = filter_visible(
            vec![tool("list_feeds"), tool("not_registered_anywhere")],
            Scope::Write,
            &all_on(),
        );
        let names: Vec<String> = kept.iter().map(|t| t.name.to_string()).collect();
        assert_eq!(names, vec!["list_feeds".to_string()]);
    }

    /// 现有只读工具都必须登记且不带危险标记；写工具（T3 的 5 个 + T4 的 8 个）
    /// 按「是否破坏性/不可逆」标 dangerous——危险集合只有 `unsubscribe` / `folder_delete`。
    #[test]
    fn read_tools_are_registered() {
        assert_eq!(TOOL_SPECS.len(), 20);
        assert_eq!(read_tool_count(), 7);
        for name in [
            "list_feeds",
            "list_folders",
            "list_articles",
            "get_article",
            "search_articles",
            "get_unread_summary",
            "db_stats",
        ] {
            let spec = spec(name).unwrap_or_else(|| panic!("{name} 未登记"));
            assert_eq!(spec.scope, Scope::Read, "{name} 应为只读");
            assert!(!spec.dangerous, "{name} 不是危险工具");
        }
        for name in [
            "set_read",
            "set_starred",
            "set_read_later",
            "refresh",
            "fetch_fulltext",
        ] {
            let spec = spec(name).unwrap_or_else(|| panic!("{name} 未登记"));
            assert_eq!(spec.scope, Scope::Write, "{name} 应为写工具");
            assert!(!spec.dangerous, "{name} 可逆且幂等，不是危险工具");
        }
        // T4：危险集合恰好只有这两个（退订级联删条目 / 删分组）
        for name in ["subscribe", "update_feed", "folder_create", "folder_rename", "import_opml", "export_opml"] {
            let spec = spec(name).unwrap_or_else(|| panic!("{name} 未登记"));
            assert_eq!(spec.scope, Scope::Write, "{name} 应为写工具");
            assert!(!spec.dangerous, "{name} 不删数据，不是危险工具");
        }
        for name in ["unsubscribe", "folder_delete"] {
            let spec = spec(name).unwrap_or_else(|| panic!("{name} 未登记"));
            assert_eq!(spec.scope, Scope::Write, "{name} 应为写工具");
            assert!(spec.dangerous, "{name} 是危险工具（需 confirm + 危险开关）");
        }
        let dangerous: Vec<&str> = TOOL_SPECS
            .iter()
            .filter(|s| s.dangerous)
            .map(|s| s.name)
            .collect();
        assert_eq!(dangerous, vec!["folder_delete", "unsubscribe"]);
        assert!(spec("no_such_tool").is_none());
    }
}
