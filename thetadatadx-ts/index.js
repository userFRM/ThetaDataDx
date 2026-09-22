/* eslint-disable */
// @ts-nocheck
/* auto-generated — do not edit by hand */

const { readFileSync } = require('fs')
let nativeBinding = null
// Which artifact actually loaded. The WASI fallback chain overwrites it with
// the flavor it resolved; the late native retry below leaves it alone because
// it only runs while no WASI candidate has been loaded.
let __napiLoadedBindingTarget = 'native'
const loadErrors = []

const isMusl = () => {
  let musl = false
  if (process.platform === 'linux') {
    musl = isMuslFromFilesystem()
    if (musl === null) {
      musl = isMuslFromReport()
    }
    if (musl === null) {
      musl = isMuslFromChildProcess()
    }
  }
  return musl
}

const isFileMusl = (f) => f.includes('libc.musl-') || f.includes('ld-musl-')

const isMuslFromFilesystem = () => {
  try {
    return readFileSync('/usr/bin/ldd', 'utf-8').includes('musl')
  } catch {
    return null
  }
}

const isMuslFromReport = () => {
  let report = null
  if (process.report && typeof process.report.getReport === 'function') {
    process.report.excludeNetwork = true
    report = process.report.getReport()
  }
  if (!report) {
    return null
  }
  if (report.header && report.header.glibcVersionRuntime) {
    return false
  }
  if (Array.isArray(report.sharedObjects)) {
    if (report.sharedObjects.some(isFileMusl)) {
      return true
    }
  }
  return false
}

const isMuslFromChildProcess = () => {
  try {
    return require('child_process').execSync('ldd --version', { encoding: 'utf8' }).includes('musl')
  } catch (e) {
    // If we reach this case, we don't know if the system is musl or not, so is better to just fallback to false
    return false
  }
}

function requireNative() {
  if (process.env.NAPI_RS_NATIVE_LIBRARY_PATH) {
    try {
      const overrideBinding = require(process.env.NAPI_RS_NATIVE_LIBRARY_PATH)
      // The override may be a generated WASI loader, which already reports its
      // own flavor. Adopt it: `module.exports` aliases this object, so claiming
      // 'native' would both misreport the artifact and overwrite the loader's
      // marker through the alias.
      __napiLoadedBindingTarget =
        overrideBinding && typeof overrideBinding.__napiBindingTarget === 'string'
          ? overrideBinding.__napiBindingTarget
          : 'native'
      return overrideBinding
    } catch (err) {
      loadErrors.push(err)
    }
  } else if (process.platform === 'android') {
    if (process.arch === 'arm64') {
      try {
        return require('./thetadatadx-ts.android-arm64.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-android-arm64')
        const bindingPackageVersion = require('thetadatadx-ts-android-arm64/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else if (process.arch === 'arm') {
      try {
        return require('./thetadatadx-ts.android-arm-eabi.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-android-arm-eabi')
        const bindingPackageVersion = require('thetadatadx-ts-android-arm-eabi/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else {
      loadErrors.push(new Error(`Unsupported architecture on Android ${process.arch}`))
    }
  } else if (process.platform === 'win32') {
    if (process.arch === 'x64') {
      if ((process.config && process.config.variables && process.config.variables.shlib_suffix === 'dll.a') || (process.config && process.config.variables && process.config.variables.node_target_type === 'shared_library')) {
        try {
          return require('./thetadatadx-ts.win32-x64-gnu.node')
        } catch (e) {
          loadErrors.push(e)
        }
        try {
          const binding = require('thetadatadx-ts-win32-x64-gnu')
          const bindingPackageVersion = require('thetadatadx-ts-win32-x64-gnu/package.json').version
          if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
            throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
          return binding
        } catch (e) {
          loadErrors.push(e)
        }
      } else {
        try {
          return require('./thetadatadx-ts.win32-x64-msvc.node')
        } catch (e) {
          loadErrors.push(e)
        }
        try {
          const binding = require('thetadatadx-ts-win32-x64-msvc')
          const bindingPackageVersion = require('thetadatadx-ts-win32-x64-msvc/package.json').version
          if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
            throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
          return binding
        } catch (e) {
          loadErrors.push(e)
        }
      }
    } else if (process.arch === 'ia32') {
      try {
        return require('./thetadatadx-ts.win32-ia32-msvc.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-win32-ia32-msvc')
        const bindingPackageVersion = require('thetadatadx-ts-win32-ia32-msvc/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else if (process.arch === 'arm64') {
      try {
        return require('./thetadatadx-ts.win32-arm64-msvc.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-win32-arm64-msvc')
        const bindingPackageVersion = require('thetadatadx-ts-win32-arm64-msvc/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else {
      loadErrors.push(new Error(`Unsupported architecture on Windows: ${process.arch}`))
    }
  } else if (process.platform === 'darwin') {
    try {
      return require('./thetadatadx-ts.darwin-universal.node')
    } catch (e) {
      loadErrors.push(e)
    }
    try {
      const binding = require('thetadatadx-ts-darwin-universal')
      const bindingPackageVersion = require('thetadatadx-ts-darwin-universal/package.json').version
      if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
        throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
      }
      return binding
    } catch (e) {
      loadErrors.push(e)
    }
    if (process.arch === 'x64') {
      try {
        return require('./thetadatadx-ts.darwin-x64.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-darwin-x64')
        const bindingPackageVersion = require('thetadatadx-ts-darwin-x64/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else if (process.arch === 'arm64') {
      try {
        return require('./thetadatadx-ts.darwin-arm64.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-darwin-arm64')
        const bindingPackageVersion = require('thetadatadx-ts-darwin-arm64/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else {
      loadErrors.push(new Error(`Unsupported architecture on macOS: ${process.arch}`))
    }
  } else if (process.platform === 'freebsd') {
    if (process.arch === 'x64') {
      try {
        return require('./thetadatadx-ts.freebsd-x64.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-freebsd-x64')
        const bindingPackageVersion = require('thetadatadx-ts-freebsd-x64/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else if (process.arch === 'arm64') {
      try {
        return require('./thetadatadx-ts.freebsd-arm64.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-freebsd-arm64')
        const bindingPackageVersion = require('thetadatadx-ts-freebsd-arm64/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else {
      loadErrors.push(new Error(`Unsupported architecture on FreeBSD: ${process.arch}`))
    }
  } else if (process.platform === 'linux') {
    if (process.arch === 'x64') {
      if (isMusl()) {
        try {
          return require('./thetadatadx-ts.linux-x64-musl.node')
        } catch (e) {
          loadErrors.push(e)
        }
        try {
          const binding = require('thetadatadx-ts-linux-x64-musl')
          const bindingPackageVersion = require('thetadatadx-ts-linux-x64-musl/package.json').version
          if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
            throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
          return binding
        } catch (e) {
          loadErrors.push(e)
        }
      } else {
        try {
          return require('./thetadatadx-ts.linux-x64-gnu.node')
        } catch (e) {
          loadErrors.push(e)
        }
        try {
          const binding = require('thetadatadx-ts-linux-x64-gnu')
          const bindingPackageVersion = require('thetadatadx-ts-linux-x64-gnu/package.json').version
          if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
            throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
          return binding
        } catch (e) {
          loadErrors.push(e)
        }
      }
    } else if (process.arch === 'arm64') {
      if (isMusl()) {
        try {
          return require('./thetadatadx-ts.linux-arm64-musl.node')
        } catch (e) {
          loadErrors.push(e)
        }
        try {
          const binding = require('thetadatadx-ts-linux-arm64-musl')
          const bindingPackageVersion = require('thetadatadx-ts-linux-arm64-musl/package.json').version
          if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
            throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
          return binding
        } catch (e) {
          loadErrors.push(e)
        }
      } else {
        try {
          return require('./thetadatadx-ts.linux-arm64-gnu.node')
        } catch (e) {
          loadErrors.push(e)
        }
        try {
          const binding = require('thetadatadx-ts-linux-arm64-gnu')
          const bindingPackageVersion = require('thetadatadx-ts-linux-arm64-gnu/package.json').version
          if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
            throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
          return binding
        } catch (e) {
          loadErrors.push(e)
        }
      }
    } else if (process.arch === 'arm') {
      if (isMusl()) {
        try {
          return require('./thetadatadx-ts.linux-arm-musleabihf.node')
        } catch (e) {
          loadErrors.push(e)
        }
        try {
          const binding = require('thetadatadx-ts-linux-arm-musleabihf')
          const bindingPackageVersion = require('thetadatadx-ts-linux-arm-musleabihf/package.json').version
          if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
            throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
          return binding
        } catch (e) {
          loadErrors.push(e)
        }
      } else {
        try {
          return require('./thetadatadx-ts.linux-arm-gnueabihf.node')
        } catch (e) {
          loadErrors.push(e)
        }
        try {
          const binding = require('thetadatadx-ts-linux-arm-gnueabihf')
          const bindingPackageVersion = require('thetadatadx-ts-linux-arm-gnueabihf/package.json').version
          if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
            throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
          return binding
        } catch (e) {
          loadErrors.push(e)
        }
      }
    } else if (process.arch === 'loong64') {
      if (isMusl()) {
        try {
          return require('./thetadatadx-ts.linux-loong64-musl.node')
        } catch (e) {
          loadErrors.push(e)
        }
        try {
          const binding = require('thetadatadx-ts-linux-loong64-musl')
          const bindingPackageVersion = require('thetadatadx-ts-linux-loong64-musl/package.json').version
          if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
            throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
          return binding
        } catch (e) {
          loadErrors.push(e)
        }
      } else {
        try {
          return require('./thetadatadx-ts.linux-loong64-gnu.node')
        } catch (e) {
          loadErrors.push(e)
        }
        try {
          const binding = require('thetadatadx-ts-linux-loong64-gnu')
          const bindingPackageVersion = require('thetadatadx-ts-linux-loong64-gnu/package.json').version
          if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
            throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
          return binding
        } catch (e) {
          loadErrors.push(e)
        }
      }
    } else if (process.arch === 'riscv64') {
      if (isMusl()) {
        try {
          return require('./thetadatadx-ts.linux-riscv64-musl.node')
        } catch (e) {
          loadErrors.push(e)
        }
        try {
          const binding = require('thetadatadx-ts-linux-riscv64-musl')
          const bindingPackageVersion = require('thetadatadx-ts-linux-riscv64-musl/package.json').version
          if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
            throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
          return binding
        } catch (e) {
          loadErrors.push(e)
        }
      } else {
        try {
          return require('./thetadatadx-ts.linux-riscv64-gnu.node')
        } catch (e) {
          loadErrors.push(e)
        }
        try {
          const binding = require('thetadatadx-ts-linux-riscv64-gnu')
          const bindingPackageVersion = require('thetadatadx-ts-linux-riscv64-gnu/package.json').version
          if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
            throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
          return binding
        } catch (e) {
          loadErrors.push(e)
        }
      }
    } else if (process.arch === 'ppc64') {
      try {
        return require('./thetadatadx-ts.linux-ppc64-gnu.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-linux-ppc64-gnu')
        const bindingPackageVersion = require('thetadatadx-ts-linux-ppc64-gnu/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else if (process.arch === 's390x') {
      try {
        return require('./thetadatadx-ts.linux-s390x-gnu.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-linux-s390x-gnu')
        const bindingPackageVersion = require('thetadatadx-ts-linux-s390x-gnu/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else {
      loadErrors.push(new Error(`Unsupported architecture on Linux: ${process.arch}`))
    }
  } else if (process.platform === 'openharmony') {
    if (process.arch === 'arm64') {
      try {
        return require('./thetadatadx-ts.openharmony-arm64.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-openharmony-arm64')
        const bindingPackageVersion = require('thetadatadx-ts-openharmony-arm64/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else if (process.arch === 'x64') {
      try {
        return require('./thetadatadx-ts.openharmony-x64.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-openharmony-x64')
        const bindingPackageVersion = require('thetadatadx-ts-openharmony-x64/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else if (process.arch === 'arm') {
      try {
        return require('./thetadatadx-ts.openharmony-arm.node')
      } catch (e) {
        loadErrors.push(e)
      }
      try {
        const binding = require('thetadatadx-ts-openharmony-arm')
        const bindingPackageVersion = require('thetadatadx-ts-openharmony-arm/package.json').version
        if (bindingPackageVersion !== '0.5.0' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          throw new Error(`Native binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
        }
        return binding
      } catch (e) {
        loadErrors.push(e)
      }
    } else {
      loadErrors.push(new Error(`Unsupported architecture on OpenHarmony: ${process.arch}`))
    }
  } else {
    loadErrors.push(new Error(`Unsupported OS: ${process.platform}, architecture: ${process.arch}`))
  }
}

function createLoadErrorChain(errors) {
  return errors.reduce((previous, current) => {
    let message
    try {
      message =
        current && typeof current.message === 'string'
          ? current.message
          : String(current)
    } catch {
      message = 'Unknown error'
    }
    const error = new Error(message)
    error.cause = previous
    return error
  }, null)
}

// NAPI_RS_FORCE_WASI is a tri-state flag:
//   unset / any other value → native binding preferred, WASI is only a fallback
//   'true'                   → prefer WASI, but retain native as a lazy fallback
//   'error'                  → require WASI without initializing a native fallback
// Treating any non-empty string as truthy (the historical behavior) meant
// NAPI_RS_FORCE_WASI=false, NAPI_RS_FORCE_WASI=0, etc. inadvertently triggered
// the WASI path, causing ENOENT for packages shipped without a .wasi.cjs file.
//
// NAPI_RS_WASI_FLAVOR selects one exact generated flavor and implies strict
// WASI loading. It never crosses into another flavor or falls back to native.
const __napiWasiFlavors = ['wasm32-wasi']
const __napiWasiFlavor = process.env.NAPI_RS_WASI_FLAVOR
const __napiWasiFlavorRequested =
  typeof __napiWasiFlavor === 'string' && __napiWasiFlavor.length > 0
if (
  __napiWasiFlavorRequested &&
  __napiWasiFlavors.indexOf(__napiWasiFlavor) === -1
) {
  throw new Error(
    'Unsupported WASI flavor "' +
      __napiWasiFlavor +
      '". Available flavors: ' +
      __napiWasiFlavors.join(', '),
  )
}
const forceWasiError = process.env.NAPI_RS_FORCE_WASI === 'error'
const forceWasi =
  process.env.NAPI_RS_FORCE_WASI === 'true' ||
  forceWasiError ||
  __napiWasiFlavorRequested

if (!forceWasi) {
  nativeBinding = requireNative()
}

if (!nativeBinding || forceWasi) {
  let wasiBinding = null
  let wasiBindingLoaded = false
  const wasiBindingErrors = []
  const __napiWasiResolveCandidate = (specifier, isPackage, localArtifacts) => {
    try {
      require.resolve(specifier)
    } catch (resolveError) {
      if (!resolveError || resolveError.code !== 'MODULE_NOT_FOUND') {
        throw resolveError
      }
      if (isPackage) {
        try {
          require.resolve(specifier + '/package.json')
        } catch (packageError) {
          if (packageError && packageError.code === 'MODULE_NOT_FOUND') {
            return resolveError
          }
          // An exports restriction proves the package exists even when its
          // package.json is not public. Preserve the root resolution failure.
          throw resolveError
        }
        // The package exists but its main/export target is broken.
        throw resolveError
      }
      return resolveError
    }
    if (localArtifacts) {
      let artifactError = null
      for (let i = 0; i < localArtifacts.length; i++) {
        try {
          require.resolve(localArtifacts[i])
          return null
        } catch (resolveError) {
          if (!resolveError || resolveError.code !== 'MODULE_NOT_FOUND') {
            throw resolveError
          }
          artifactError = resolveError
        }
      }
      return artifactError
    }
    return null
  }
  if (!wasiBindingLoaded && (!__napiWasiFlavorRequested || __napiWasiFlavor === 'wasm32-wasi')) {
    let candidateError = null
    let candidateFailed = false
    try {
      candidateError = __napiWasiResolveCandidate('./thetadatadx-ts.wasi.cjs', false, ['./thetadatadx-ts.wasm32-wasi.debug.wasm', './thetadatadx-ts.wasm32-wasi.wasm'])
      candidateFailed = candidateError !== null
      if (!candidateFailed) {
        wasiBinding = require('./thetadatadx-ts.wasi.cjs')
        nativeBinding = wasiBinding
        __napiLoadedBindingTarget = 'wasm32-wasi'
        wasiBindingLoaded = true
      }
    } catch (err) {
      candidateError = err
      candidateFailed = true
    }
    if (candidateFailed) {
      wasiBindingErrors.push(candidateError)
      loadErrors.push(candidateError)
    }
  }
  if (!wasiBindingLoaded && (!__napiWasiFlavorRequested || __napiWasiFlavor === 'wasm32-wasi')) {
    let candidateError = null
    let candidateFailed = false
    try {
      candidateError = __napiWasiResolveCandidate('thetadatadx-ts-wasm32-wasi', true, undefined)
      candidateFailed = candidateError !== null
      if (!candidateFailed) {
        if (process.env.NAPI_RS_ENFORCE_VERSION_CHECK && process.env.NAPI_RS_ENFORCE_VERSION_CHECK !== '0') {
          const bindingPackageVersion = require('thetadatadx-ts-wasm32-wasi/package.json').version
          if (bindingPackageVersion !== '0.5.0') {
            throw new Error(`WASI binding package version mismatch, expected 0.5.0 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)
          }
        }
        wasiBinding = require('thetadatadx-ts-wasm32-wasi')
        nativeBinding = wasiBinding
        __napiLoadedBindingTarget = 'wasm32-wasi'
        wasiBindingLoaded = true
      }
    } catch (err) {
      candidateError = err
      candidateFailed = true
    }
    if (candidateFailed) {
      wasiBindingErrors.push(candidateError)
      loadErrors.push(candidateError)
    }
  }
  if (
    !wasiBindingLoaded &&
    forceWasi &&
    !forceWasiError &&
    !__napiWasiFlavorRequested
  ) {
    nativeBinding = requireNative()
  }
  if ((forceWasiError || __napiWasiFlavorRequested) && !wasiBindingLoaded) {
    const error = new Error(
      __napiWasiFlavorRequested
        ? 'WASI binding for flavor "' + __napiWasiFlavor + '" not found'
        : 'WASI binding not found and NAPI_RS_FORCE_WASI is set to error',
    )
    error.cause = createLoadErrorChain(wasiBindingErrors)
    throw error
  }
}

if (!nativeBinding) {
  if (loadErrors.length > 0) {
    const error = new Error(
      `Cannot find native binding. ` +
        `npm has a bug related to optional dependencies (https://github.com/npm/cli/issues/4828). ` +
        'Please try `npm i` again after removing both package-lock.json and node_modules directory.',
    )
    // assign instead of the `new Error(message, { cause })` options form,
    // which Node < 16.9 silently ignores
    error.cause = createLoadErrorChain(loadErrors)
    throw error
  }
  throw new Error(`Failed to load native binding`)
}

function __napiStampBindingTarget(exportsObject, target) {
  if (
    Object.prototype.hasOwnProperty.call(exportsObject, '__napiBindingTarget')
  ) {
    if (exportsObject.__napiBindingTarget === target) {
      // Already ours: the root entry aliases the object it loaded, so a WASI
      // fallback candidate — or a `NAPI_RS_NATIVE_LIBRARY_PATH` override that
      // is a generated loader — arrives already stamped with this same value.
      return target
    }
    const error = new Error(
      '`__napiBindingTarget` is reserved by the generated binding loader, but the loaded binding already exports it. Rename the export, e.g. #[napi(js_name = "...")].',
    )
    error.code = 'ERR_NAPI_BINDING_TARGET_CONFLICT'
    throw error
  }
  if (!Object.isExtensible(exportsObject)) {
    // A `#[napi(module_exports)]` hook may seal or freeze this object
    // (`Object::seal` / `Object::freeze`). Reporting the artifact is metadata,
    // never a reason to fail an otherwise successful load, so the stamp is
    // skipped. What a consumer still sees then follows the entry point: the
    // browser and deferred loaders declare `__napiBindingTarget` at module
    // level and go on reporting it, while the CommonJS entries hand back this
    // very object as `module.exports`, so there the value is absent.
    return target
  }
  try {
    // [[Define]], not [[Set]]: an ordinary assignment walks the prototype
    // chain, so an inherited accessor could swallow the value or throw and
    // fail an otherwise successful load. The descriptor is what a successful
    // assignment would have produced.
    Object.defineProperty(exportsObject, '__napiBindingTarget', {
      configurable: true,
      enumerable: true,
      value: target,
      writable: true,
    })
  } catch {
    // Same rule as the non-extensible skip above: reporting the artifact is
    // metadata, never a reason to fail an otherwise successful load. An exotic
    // object (a Proxy whose defineProperty trap refuses) is skipped, not
    // thrown over.
  }
  // The CommonJS loaders assign this return value so `cjs-module-lexer` — and
  // therefore Node's CJS -> ESM named export detection — can see
  // `__napiBindingTarget` statically.
  return target
}
// Stamp before the alias, not after. The guard only reads `nativeBinding`
// (`hasOwnProperty` plus a comparison), which is safe against any addon
// accessor; an assignment is not, because a `#[napi(module_exports)]` hook can
// expose a getter reporting this very value and a setter that throws. So the
// assignment lands on the loader's own `module.exports`, still the original
// object here, and the alias below replaces it.
//
// The assignment is what keeps the marker a statically visible CommonJS export:
// `cjs-module-lexer` is Node's CJS -> ESM named export detection, it cannot see
// a bare call, and the later `module.exports = nativeBinding` does not undo the
// detection. The assignment itself always succeeds — its target is this
// loader's own, still extensible `module.exports` — and the alias below then
// discards the value it wrote. What a consumer reads is whatever the guard put
// on `nativeBinding`, so on a frozen binding, where the guard skips, the
// linked import resolves to `undefined`.
module.exports.__napiBindingTarget = __napiStampBindingTarget(nativeBinding, __napiLoadedBindingTarget)
module.exports = nativeBinding
module.exports.Client = nativeBinding.Client
module.exports.Config = nativeBinding.Config
module.exports.ContractRef = nativeBinding.ContractRef
// `Contract` is the public name for the fluent contract builder; it
// aliases the `ContractRef` constructor so
// `require('thetadatadx-ts').Contract.stock(...)` resolves.
module.exports.Contract = nativeBinding.ContractRef;
module.exports.Credentials = nativeBinding.Credentials
module.exports.FlatFileRowList = nativeBinding.FlatFileRowList
module.exports.FlatFilesNamespace = nativeBinding.FlatFilesNamespace
module.exports.MarketDataClient = nativeBinding.MarketDataClient
module.exports.MarketDataView = nativeBinding.MarketDataView
module.exports.RecordBatchStreamHandle = nativeBinding.RecordBatchStreamHandle
module.exports.SecType = nativeBinding.SecType
module.exports.StreamingClient = nativeBinding.StreamingClient
module.exports.StreamView = nativeBinding.StreamView
module.exports.Subscription = nativeBinding.Subscription
module.exports.Util = nativeBinding.Util
module.exports.__benchFloodEvents = nativeBinding.__benchFloodEvents
module.exports.__benchFloodEventsArrowIpc = nativeBinding.__benchFloodEventsArrowIpc
module.exports.__benchFloodEventsBatched = nativeBinding.__benchFloodEventsBatched
module.exports.calendarDayPresentColumns = nativeBinding.calendarDayPresentColumns
module.exports.calendarDayToArrowIpc = nativeBinding.calendarDayToArrowIpc
module.exports.calendarDayToArrowIpcProjected = nativeBinding.calendarDayToArrowIpcProjected
module.exports.eodTickPresentColumns = nativeBinding.eodTickPresentColumns
module.exports.eodTickToArrowIpc = nativeBinding.eodTickToArrowIpc
module.exports.eodTickToArrowIpcProjected = nativeBinding.eodTickToArrowIpcProjected
module.exports.greeksAllTickPresentColumns = nativeBinding.greeksAllTickPresentColumns
module.exports.greeksAllTickToArrowIpc = nativeBinding.greeksAllTickToArrowIpc
module.exports.greeksAllTickToArrowIpcProjected = nativeBinding.greeksAllTickToArrowIpcProjected
module.exports.greeksEodTickPresentColumns = nativeBinding.greeksEodTickPresentColumns
module.exports.greeksEodTickToArrowIpc = nativeBinding.greeksEodTickToArrowIpc
module.exports.greeksEodTickToArrowIpcProjected = nativeBinding.greeksEodTickToArrowIpcProjected
module.exports.greeksFirstOrderTickPresentColumns = nativeBinding.greeksFirstOrderTickPresentColumns
module.exports.greeksFirstOrderTickToArrowIpc = nativeBinding.greeksFirstOrderTickToArrowIpc
module.exports.greeksFirstOrderTickToArrowIpcProjected = nativeBinding.greeksFirstOrderTickToArrowIpcProjected
module.exports.greeksSecondOrderTickPresentColumns = nativeBinding.greeksSecondOrderTickPresentColumns
module.exports.greeksSecondOrderTickToArrowIpc = nativeBinding.greeksSecondOrderTickToArrowIpc
module.exports.greeksSecondOrderTickToArrowIpcProjected = nativeBinding.greeksSecondOrderTickToArrowIpcProjected
module.exports.greeksThirdOrderTickPresentColumns = nativeBinding.greeksThirdOrderTickPresentColumns
module.exports.greeksThirdOrderTickToArrowIpc = nativeBinding.greeksThirdOrderTickToArrowIpc
module.exports.greeksThirdOrderTickToArrowIpcProjected = nativeBinding.greeksThirdOrderTickToArrowIpcProjected
module.exports.indexPriceAtTimeTickPresentColumns = nativeBinding.indexPriceAtTimeTickPresentColumns
module.exports.indexPriceAtTimeTickToArrowIpc = nativeBinding.indexPriceAtTimeTickToArrowIpc
module.exports.indexPriceAtTimeTickToArrowIpcProjected = nativeBinding.indexPriceAtTimeTickToArrowIpcProjected
module.exports.interestRateTickPresentColumns = nativeBinding.interestRateTickPresentColumns
module.exports.interestRateTickToArrowIpc = nativeBinding.interestRateTickToArrowIpc
module.exports.interestRateTickToArrowIpcProjected = nativeBinding.interestRateTickToArrowIpcProjected
module.exports.Interval = nativeBinding.Interval
module.exports.ivTickPresentColumns = nativeBinding.ivTickPresentColumns
module.exports.ivTickToArrowIpc = nativeBinding.ivTickToArrowIpc
module.exports.ivTickToArrowIpcProjected = nativeBinding.ivTickToArrowIpcProjected
module.exports.marketValueTickPresentColumns = nativeBinding.marketValueTickPresentColumns
module.exports.marketValueTickToArrowIpc = nativeBinding.marketValueTickToArrowIpc
module.exports.marketValueTickToArrowIpcProjected = nativeBinding.marketValueTickToArrowIpcProjected
module.exports.ohlcTickPresentColumns = nativeBinding.ohlcTickPresentColumns
module.exports.ohlcTickToArrowIpc = nativeBinding.ohlcTickToArrowIpc
module.exports.ohlcTickToArrowIpcProjected = nativeBinding.ohlcTickToArrowIpcProjected
module.exports.openInterestTickPresentColumns = nativeBinding.openInterestTickPresentColumns
module.exports.openInterestTickToArrowIpc = nativeBinding.openInterestTickToArrowIpc
module.exports.openInterestTickToArrowIpcProjected = nativeBinding.openInterestTickToArrowIpcProjected
module.exports.optionContractPresentColumns = nativeBinding.optionContractPresentColumns
module.exports.optionContractToArrowIpc = nativeBinding.optionContractToArrowIpc
module.exports.optionContractToArrowIpcProjected = nativeBinding.optionContractToArrowIpcProjected
module.exports.priceTickPresentColumns = nativeBinding.priceTickPresentColumns
module.exports.priceTickToArrowIpc = nativeBinding.priceTickToArrowIpc
module.exports.priceTickToArrowIpcProjected = nativeBinding.priceTickToArrowIpcProjected
module.exports.quoteTickPresentColumns = nativeBinding.quoteTickPresentColumns
module.exports.quoteTickToArrowIpc = nativeBinding.quoteTickToArrowIpc
module.exports.quoteTickToArrowIpcProjected = nativeBinding.quoteTickToArrowIpcProjected
module.exports.RateType = nativeBinding.RateType
module.exports.RequestType = nativeBinding.RequestType
module.exports.Right = nativeBinding.Right
module.exports.tradeGreeksAllTickPresentColumns = nativeBinding.tradeGreeksAllTickPresentColumns
module.exports.tradeGreeksAllTickToArrowIpc = nativeBinding.tradeGreeksAllTickToArrowIpc
module.exports.tradeGreeksAllTickToArrowIpcProjected = nativeBinding.tradeGreeksAllTickToArrowIpcProjected
module.exports.tradeGreeksFirstOrderTickPresentColumns = nativeBinding.tradeGreeksFirstOrderTickPresentColumns
module.exports.tradeGreeksFirstOrderTickToArrowIpc = nativeBinding.tradeGreeksFirstOrderTickToArrowIpc
module.exports.tradeGreeksFirstOrderTickToArrowIpcProjected = nativeBinding.tradeGreeksFirstOrderTickToArrowIpcProjected
module.exports.tradeGreeksImpliedVolatilityTickPresentColumns = nativeBinding.tradeGreeksImpliedVolatilityTickPresentColumns
module.exports.tradeGreeksImpliedVolatilityTickToArrowIpc = nativeBinding.tradeGreeksImpliedVolatilityTickToArrowIpc
module.exports.tradeGreeksImpliedVolatilityTickToArrowIpcProjected = nativeBinding.tradeGreeksImpliedVolatilityTickToArrowIpcProjected
module.exports.tradeGreeksSecondOrderTickPresentColumns = nativeBinding.tradeGreeksSecondOrderTickPresentColumns
module.exports.tradeGreeksSecondOrderTickToArrowIpc = nativeBinding.tradeGreeksSecondOrderTickToArrowIpc
module.exports.tradeGreeksSecondOrderTickToArrowIpcProjected = nativeBinding.tradeGreeksSecondOrderTickToArrowIpcProjected
module.exports.tradeGreeksThirdOrderTickPresentColumns = nativeBinding.tradeGreeksThirdOrderTickPresentColumns
module.exports.tradeGreeksThirdOrderTickToArrowIpc = nativeBinding.tradeGreeksThirdOrderTickToArrowIpc
module.exports.tradeGreeksThirdOrderTickToArrowIpcProjected = nativeBinding.tradeGreeksThirdOrderTickToArrowIpcProjected
module.exports.tradeQuoteTickPresentColumns = nativeBinding.tradeQuoteTickPresentColumns
module.exports.tradeQuoteTickToArrowIpc = nativeBinding.tradeQuoteTickToArrowIpc
module.exports.tradeQuoteTickToArrowIpcProjected = nativeBinding.tradeQuoteTickToArrowIpcProjected
module.exports.tradeTickPresentColumns = nativeBinding.tradeTickPresentColumns
module.exports.tradeTickToArrowIpc = nativeBinding.tradeTickToArrowIpc
module.exports.tradeTickToArrowIpcProjected = nativeBinding.tradeTickToArrowIpcProjected
module.exports.Venue = nativeBinding.Venue
module.exports.Version = nativeBinding.Version
