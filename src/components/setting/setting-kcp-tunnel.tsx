import {
  ContentCopyRounded,
  PlayArrowRounded,
  RestartAltRounded,
  SettingsEthernetRounded,
  StopRounded,
} from '@mui/icons-material'
import {
  Box,
  Button,
  Chip,
  Checkbox,
  FormControlLabel,
  IconButton,
  InputAdornment,
  MenuItem,
  Stack,
  Switch,
  TextField,
  Typography,
} from '@mui/material'
import { useLockFn } from 'ahooks'
import { useEffect, useMemo, useState } from 'react'

import { BaseDialog } from '@/components/base'
import { useVerge } from '@/hooks/use-verge'
import {
  getKcptunStatus,
  startKcptunClient,
  stopKcptunClient,
  testKcptunUpstreamProxy,
} from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import getSystem from '@/utils/get-system'

import { SettingItem, SettingList } from './mods/setting-comp'

const DEFAULT_KCP_PROXY: IKcpProxyConfig = {
  enabled: false,
  server: '127.0.0.1',
  remote_port: 29900,
  local_port: 1087,
  key: 'password',
  crypt: 'aes',
  mode: 'fast',
  mtu: 1350,
  sndwnd: 512,
  rcvwnd: 512,
  datashard: 10,
  parityshard: 3,
  dscp: 0,
  nocomp: true,
  tcp: false,
  catch_all: false,
  domains: [],
}

const CRYPT_OPTIONS = [
  'aes',
  'aes-128',
  'aes-128-gcm',
  'aes-192',
  'salsa20',
  'blowfish',
  'twofish',
  'cast5',
  '3des',
  'tea',
  'xtea',
  'xor',
  'sm4',
  'none',
  'null',
]

const MODE_OPTIONS = ['fast', 'fast2', 'fast3', 'normal', 'manual']

const TCP_SUPPORTED = getSystem() === 'linux'
const TCP_UNSUPPORTED_MESSAGE =
  'kcptun --tcp uses tcpraw and is only supported by Linux clients. Disable tcp on this device; non-Linux clients cannot communicate with a Linux VPS that requires kcptun --tcp/tcpraw.'

const toNumber = (value: unknown, fallback: number) => {
  const number = Number(value)
  return Number.isFinite(number) ? number : fallback
}

const toOptionalNumber = (value: string) =>
  value.trim() === '' ? undefined : Number(value)

const normalizeConfig = (value?: IKcpProxyConfig): IKcpProxyConfig => ({
  ...DEFAULT_KCP_PROXY,
  ...value,
  domains: value?.domains ?? [],
})

const toDomainText = (domains?: string[]) => (domains ?? []).join('\n')

const fromDomainText = (value: string) =>
  value
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean)

const twoColumnGridSx = {
  display: 'grid',
  gridTemplateColumns: { xs: '1fr', sm: 'repeat(2, minmax(0, 1fr))' },
  gap: 1.5,
  minWidth: 0,
}

const KcpTunnelSetting = () => {
  const { verge, patchVerge } = useVerge()
  const [open, setOpen] = useState(false)
  const [running, setRunning] = useState(false)
  const [testing, setTesting] = useState(false)
  const [value, setValue] = useState<IKcpProxyConfig>(DEFAULT_KCP_PROXY)
  const [domainText, setDomainText] = useState('')
  const [testLog, setTestLog] = useState('')

  const current = useMemo(
    () => normalizeConfig(verge?.kcp_proxy),
    [verge?.kcp_proxy],
  )

  const refreshStatus = useLockFn(async () => {
    setRunning(await getKcptunStatus())
  })

  useEffect(() => {
    refreshStatus()
  }, [refreshStatus])

  const openDialog = () => {
    setValue(current)
    setDomainText(toDomainText(current.domains))
    setTestLog('')
    setOpen(true)
    void refreshStatus()
  }

  const patchValue = (patch: Partial<IKcpProxyConfig>) => {
    setValue((prev) => ({ ...prev, ...patch }))
  }

  const patchTcp = (checked: boolean) => {
    if (checked && !TCP_SUPPORTED) {
      showNotice.error(TCP_UNSUPPORTED_MESSAGE)
      patchValue({ tcp: false })
      return
    }

    patchValue({ tcp: checked })
  }

  const buildConfig = (): IKcpProxyConfig => ({
    ...value,
    remote_port: toNumber(value.remote_port, DEFAULT_KCP_PROXY.remote_port!),
    local_port: toNumber(value.local_port, DEFAULT_KCP_PROXY.local_port!),
    mtu: toNumber(value.mtu, DEFAULT_KCP_PROXY.mtu!),
    sndwnd: toNumber(value.sndwnd, DEFAULT_KCP_PROXY.sndwnd!),
    rcvwnd: toNumber(value.rcvwnd, DEFAULT_KCP_PROXY.rcvwnd!),
    datashard: toNumber(value.datashard, DEFAULT_KCP_PROXY.datashard!),
    parityshard: toNumber(value.parityshard, DEFAULT_KCP_PROXY.parityshard!),
    dscp: toNumber(value.dscp, DEFAULT_KCP_PROXY.dscp!),
    domains: fromDomainText(domainText),
  })

  const save = useLockFn(async () => {
    try {
      const next = buildConfig()
      await patchVerge({ kcp_proxy: next })
      await refreshStatus()
      setOpen(false)
    } catch (err) {
      showNotice.error(String(err))
    }
  })

  const start = useLockFn(async () => {
    try {
      await patchVerge({ kcp_proxy: buildConfig() })
      await startKcptunClient()
      await refreshStatus()
    } catch (err) {
      showNotice.error(String(err))
    }
  })

  const stop = useLockFn(async () => {
    await stopKcptunClient()
    await refreshStatus()
  })

  const restart = useLockFn(async () => {
    try {
      await patchVerge({ kcp_proxy: buildConfig() })
      await refreshStatus()
    } catch (err) {
      showNotice.error(String(err))
    }
  })

  const copyProxyPassword = useLockFn(async () => {
    try {
      await navigator.clipboard.writeText(value.proxy_password ?? '')
      showNotice.success(
        'shared.feedback.notifications.common.copySuccess',
        1000,
      )
    } catch (err) {
      showNotice.error(String(err))
    }
  })

  const testUpstreamProxy = useLockFn(async () => {
    setTesting(true)
    setTestLog(
      [
        '* Preparing KCP Tunnel upstream proxy test',
        '* Saving dialog values to app config',
      ].join('\n'),
    )
    try {
      await patchVerge({ kcp_proxy: buildConfig() })
      setTestLog((prev) => `${prev}\n* Ensuring kcptun client is running`)
      await startKcptunClient()
      await refreshStatus()
      setTestLog((prev) => `${prev}\n* Running CONNECT/TLS/HTTP probe`)
      const logs = await testKcptunUpstreamProxy()
      setTestLog((prev) => `${prev}\n${logs}`)
    } catch (err) {
      setTestLog((prev) =>
        [prev, '', `! ${String(err)}`].filter(Boolean).join('\n'),
      )
      showNotice.error(String(err))
    } finally {
      setTesting(false)
    }
  })

  return (
    <SettingList title="KCP Tunnel">
      <SettingItem
        label="KCP Tunnel"
        extra={
          <Chip
            size="small"
            color={
              running ? 'success' : current.enabled ? 'warning' : 'default'
            }
            label={
              running ? 'Running' : current.enabled ? 'Enabled' : 'Disabled'
            }
            sx={{ ml: 1 }}
          />
        }
        secondary={`127.0.0.1:${current.local_port} -> ${current.server}:${current.remote_port}`}
        onClick={openDialog}
      />

      <BaseDialog
        open={open}
        title="KCP Tunnel"
        okBtn="Save"
        cancelBtn="Cancel"
        loading={testing}
        onOk={save}
        onCancel={() => setOpen(false)}
        onClose={() => setOpen(false)}
        contentSx={{
          width: { xs: 'calc(100vw - 96px)', sm: 520 },
          maxWidth: '100%',
          boxSizing: 'border-box',
          overflowX: 'hidden',
        }}
      >
        <Stack spacing={2} sx={{ pt: 1 }}>
          <Stack
            direction="row"
            spacing={1}
            useFlexGap
            sx={{ alignItems: 'center', flexWrap: 'wrap' }}
          >
            <Switch
              checked={!!value.enabled}
              onChange={(_, checked) => patchValue({ enabled: checked })}
            />
            <Chip
              size="small"
              color={running ? 'success' : 'default'}
              label={running ? 'Running' : 'Stopped'}
            />
            <Box sx={{ flex: 1 }} />
            <IconButton
              size="small"
              title="Start"
              disabled={testing || !value.enabled}
              onClick={start}
            >
              <PlayArrowRounded />
            </IconButton>
            <IconButton
              size="small"
              title="Stop"
              disabled={testing}
              onClick={stop}
            >
              <StopRounded />
            </IconButton>
            <IconButton
              size="small"
              title="Restart"
              disabled={testing || !value.enabled}
              onClick={restart}
            >
              <RestartAltRounded />
            </IconButton>
          </Stack>

          <TextField
            size="small"
            label="Client binary"
            value={value.client_path ?? ''}
            onChange={(e) => patchValue({ client_path: e.target.value })}
            fullWidth
          />

          <Box sx={twoColumnGridSx}>
            <Box sx={{ minWidth: 0, gridColumn: '1 / -1' }}>
              <TextField
                size="small"
                label="Server"
                value={value.server ?? ''}
                onChange={(e) => patchValue({ server: e.target.value })}
                fullWidth
              />
            </Box>
            <Box sx={{ minWidth: 0 }}>
              <TextField
                size="small"
                label="Remote port"
                type="number"
                value={value.remote_port ?? ''}
                onChange={(e) =>
                  patchValue({ remote_port: toOptionalNumber(e.target.value) })
                }
                fullWidth
              />
            </Box>
            <Box sx={{ minWidth: 0 }}>
              <TextField
                size="small"
                label="Local port"
                type="number"
                value={value.local_port ?? ''}
                onChange={(e) =>
                  patchValue({ local_port: toOptionalNumber(e.target.value) })
                }
                fullWidth
              />
            </Box>
          </Box>

          <Box sx={twoColumnGridSx}>
            <Box sx={{ minWidth: 0 }}>
              <TextField
                size="small"
                label="Key"
                value={value.key ?? ''}
                onChange={(e) => patchValue({ key: e.target.value })}
                fullWidth
              />
            </Box>
            <Box sx={{ minWidth: 0 }}>
              <TextField
                size="small"
                label="Crypt"
                select
                value={value.crypt ?? ''}
                onChange={(e) => patchValue({ crypt: e.target.value })}
                fullWidth
              >
                {CRYPT_OPTIONS.map((option) => (
                  <MenuItem key={option} value={option}>
                    {option}
                  </MenuItem>
                ))}
              </TextField>
            </Box>
            <Box sx={{ minWidth: 0 }}>
              <TextField
                size="small"
                label="Mode"
                select
                value={value.mode ?? ''}
                onChange={(e) => patchValue({ mode: e.target.value })}
                fullWidth
              >
                {MODE_OPTIONS.map((option) => (
                  <MenuItem key={option} value={option}>
                    {option}
                  </MenuItem>
                ))}
              </TextField>
            </Box>
          </Box>

          <Box sx={twoColumnGridSx}>
            {(
              [
                'mtu',
                'sndwnd',
                'rcvwnd',
                'datashard',
                'parityshard',
                'dscp',
              ] as const
            ).map((key) => (
              <Box key={key} sx={{ minWidth: 0 }}>
                <TextField
                  size="small"
                  label={key}
                  type="number"
                  value={value[key] ?? ''}
                  onChange={(e) =>
                    patchValue({ [key]: toOptionalNumber(e.target.value) })
                  }
                  fullWidth
                />
              </Box>
            ))}
          </Box>

          <Stack
            direction="row"
            spacing={1}
            useFlexGap
            sx={{ alignItems: 'center', flexWrap: 'wrap' }}
          >
            <Switch
              checked={value.nocomp ?? true}
              onChange={(_, checked) => patchValue({ nocomp: checked })}
            />
            <Chip size="small" label="nocomp" />
            <Switch
              checked={value.tcp ?? false}
              onChange={(_, checked) => patchTcp(checked)}
              title={TCP_SUPPORTED ? 'tcp' : TCP_UNSUPPORTED_MESSAGE}
            />
            <Chip
              size="small"
              color={!TCP_SUPPORTED && value.tcp ? 'error' : 'default'}
              label={TCP_SUPPORTED ? 'tcp' : 'tcp (Linux only)'}
              title={TCP_SUPPORTED ? 'tcp' : TCP_UNSUPPORTED_MESSAGE}
            />
            <Chip
              size="small"
              icon={<SettingsEthernetRounded />}
              label="CONNECT+"
            />
          </Stack>

          <Box>
            <FormControlLabel
              control={
                <Checkbox
                  checked={!!value.catch_all}
                  onChange={(_, checked) => patchValue({ catch_all: checked })}
                />
              }
              label="接管所有流量"
            />
            <Typography variant="body2" color="text.secondary">
              仅启用 KCP Tunnel 会添加一个可用代理节点；实际流量仍由当前 Clash
              模式和规则决定。规则模式下，勾选接管所有流量后会前置 MATCH
              规则，全量经由 kcptun；未勾选时仅下方目标域名会优先走 kcptun。
            </Typography>
          </Box>

          <Box sx={twoColumnGridSx}>
            <Box sx={{ minWidth: 0 }}>
              <TextField
                size="small"
                label="Proxy username"
                value={value.proxy_username ?? ''}
                onChange={(e) => patchValue({ proxy_username: e.target.value })}
                fullWidth
              />
            </Box>
            <Box sx={{ minWidth: 0 }}>
              <TextField
                size="small"
                label="Proxy password"
                type="password"
                value={value.proxy_password ?? ''}
                onChange={(e) => patchValue({ proxy_password: e.target.value })}
                fullWidth
                slotProps={{
                  input: {
                    endAdornment: (
                      <InputAdornment position="end">
                        <IconButton
                          edge="end"
                          size="small"
                          title="Copy"
                          disabled={!value.proxy_password}
                          onClick={copyProxyPassword}
                        >
                          <ContentCopyRounded fontSize="small" />
                        </IconButton>
                      </InputAdornment>
                    ),
                  },
                }}
              />
            </Box>
          </Box>

          <TextField
            size="small"
            label="Priority domains"
            value={domainText}
            onChange={(e) => setDomainText(e.target.value)}
            disabled={!!value.catch_all}
            helperText={
              value.catch_all
                ? '接管所有流量时暂不使用目标域名列表，已填写内容会保留。'
                : '每行一个域名；这些域名会优先通过 kcptun，其余流量按原 Clash 规则处理。'
            }
            multiline
            minRows={5}
            fullWidth
          />

          <Stack
            direction="row"
            spacing={1}
            useFlexGap
            sx={{ flexWrap: 'wrap' }}
          >
            <Button variant="outlined" onClick={refreshStatus}>
              Refresh Status
            </Button>
            <Button
              variant="contained"
              disabled={testing || !value.enabled}
              onClick={testUpstreamProxy}
            >
              {testing ? 'Testing...' : 'Test Upstream Proxy'}
            </Button>
          </Stack>

          <TextField
            size="small"
            label="Test log"
            value={testLog}
            multiline
            minRows={8}
            fullWidth
            slotProps={{
              input: {
                readOnly: true,
                sx: {
                  alignItems: 'flex-start',
                  fontFamily:
                    'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
                  fontSize: 12,
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-all',
                },
              },
            }}
          />
        </Stack>
      </BaseDialog>
    </SettingList>
  )
}

export default KcpTunnelSetting
