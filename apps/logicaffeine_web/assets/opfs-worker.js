// OPFS Web Worker - Safari-compatible file system operations
// Uses createSyncAccessHandle() which is the only write API Safari supports on main thread workaround
//
// This worker handles all OPFS operations synchronously within the worker context,
// allowing the main thread to send async requests via postMessage.

// Feature detection - check OPFS availability
const hasOPFS = typeof navigator !== 'undefined'
    && typeof navigator.storage !== 'undefined'
    && typeof navigator.storage.getDirectory === 'function';

if (!hasOPFS) {
    console.error('[OPFS Worker] OPFS not available:', {
        hasNavigator: typeof navigator !== 'undefined',
        hasStorage: typeof navigator !== 'undefined' && typeof navigator.storage !== 'undefined',
        hasGetDirectory: typeof navigator !== 'undefined' && navigator.storage && typeof navigator.storage.getDirectory === 'function',
        userAgent: typeof navigator !== 'undefined' ? navigator.userAgent : 'unknown'
    });
}

// Cache for directory handles to avoid repeated lookups
const dirHandleCache = new Map();

// Get the OPFS root directory
let rootPromise = null;
async function getRoot() {
    if (!hasOPFS) {
        throw new Error('OPFS not supported in this browser/context. iOS Safari Web Workers do not support OPFS.');
    }
    if (!rootPromise) {
        rootPromise = navigator.storage.getDirectory();
    }
    return rootPromise;
}

// Navigate to a directory, optionally creating intermediate directories
async function getDir(path, create) {
    const root = await getRoot();

    if (!path || path === '' || path === '/') {
        return root;
    }

    // Normalize path
    path = path.replace(/^\/+/, '').replace(/\/+$/, '');
    if (!path) {
        return root;
    }

    // Check cache first
    const cacheKey = `${path}:${create}`;
    if (!create && dirHandleCache.has(cacheKey)) {
        return dirHandleCache.get(cacheKey);
    }

    const segments = path.split('/').filter(s => s && s !== '.');
    let current = root;

    for (const segment of segments) {
        if (segment === '..') {
            continue; // Skip parent traversal for security
        }
        current = await current.getDirectoryHandle(segment, { create });
    }

    // Cache successful lookups
    if (!create) {
        dirHandleCache.set(cacheKey, current);
    }

    return current;
}

// Get parent directory path and filename from a path
function splitPath(path) {
    path = path.replace(/^\/+/, '');
    const lastSlash = path.lastIndexOf('/');
    if (lastSlash === -1) {
        return { parent: '', filename: path };
    }
    return {
        parent: path.slice(0, lastSlash),
        filename: path.slice(lastSlash + 1)
    };
}

// Operation handlers
const operations = {
    async read({ path }) {
        const { parent, filename } = splitPath(path);
        const dir = await getDir(parent, false);
        const fileHandle = await dir.getFileHandle(filename);

        // Use sync access handle for consistent behavior
        const accessHandle = await fileHandle.createSyncAccessHandle();
        try {
            const size = accessHandle.getSize();
            const buffer = new Uint8Array(size);
            accessHandle.read(buffer, { at: 0 });
            return { ok: true, data: buffer };
        } finally {
            accessHandle.close();
        }
    },

    async write({ path, data }) {
        const { parent, filename } = splitPath(path);

        // Ensure parent directory exists
        if (parent) {
            await getDir(parent, true);
        }

        const dir = await getDir(parent, true);
        const fileHandle = await dir.getFileHandle(filename, { create: true });

        const accessHandle = await fileHandle.createSyncAccessHandle();
        try {
            // Truncate and write
            accessHandle.truncate(0);
            accessHandle.write(data, { at: 0 });
            accessHandle.flush();
            return { ok: true };
        } finally {
            accessHandle.close();
        }
    },

    async append({ path, data }) {
        const { parent, filename } = splitPath(path);

        // Ensure parent directory exists
        if (parent) {
            await getDir(parent, true);
        }

        const dir = await getDir(parent, true);
        const fileHandle = await dir.getFileHandle(filename, { create: true });

        const accessHandle = await fileHandle.createSyncAccessHandle();
        try {
            const currentSize = accessHandle.getSize();
            accessHandle.write(data, { at: currentSize });
            accessHandle.flush();
            return { ok: true };
        } finally {
            accessHandle.close();
        }
    },

    async exists({ path }) {
        const { parent, filename } = splitPath(path);

        try {
            const dir = await getDir(parent, false);

            // Try as file first
            try {
                await dir.getFileHandle(filename);
                return { ok: true, exists: true, isFile: true };
            } catch (e) {
                if (e.name === 'NotFoundError' || e.name === 'TypeMismatchError') {
                    // Try as directory
                    try {
                        await dir.getDirectoryHandle(filename);
                        return { ok: true, exists: true, isFile: false };
                    } catch (e2) {
                        if (e2.name === 'NotFoundError' || e2.name === 'TypeMismatchError') {
                            return { ok: true, exists: false };
                        }
                        throw e2;
                    }
                }
                throw e;
            }
        } catch (e) {
            if (e.name === 'NotFoundError') {
                return { ok: true, exists: false };
            }
            throw e;
        }
    },

    async remove({ path }) {
        const { parent, filename } = splitPath(path);
        const dir = await getDir(parent, false);
        await dir.removeEntry(filename);

        // Invalidate cache
        dirHandleCache.clear();

        return { ok: true };
    },

    async removeRecursive({ path }) {
        const { parent, filename } = splitPath(path);
        const dir = await getDir(parent, false);
        await dir.removeEntry(filename, { recursive: true });

        // Invalidate cache
        dirHandleCache.clear();

        return { ok: true };
    },

    async createDirAll({ path }) {
        await getDir(path, true);
        return { ok: true };
    },

    async rename({ from, to }) {
        // OPFS doesn't have native rename - read, write, delete
        const readResult = await operations.read({ path: from });
        if (!readResult.ok) {
            throw new Error(`Failed to read source file: ${from}`);
        }

        await operations.write({ path: to, data: readResult.data });
        await operations.remove({ path: from });

        return { ok: true };
    },

    async listDir({ path }) {
        const dir = await getDir(path, false);
        const entries = [];

        for await (const [name, handle] of dir.entries()) {
            entries.push({
                name,
                isDirectory: handle.kind === 'directory'
            });
        }

        // Sort: directories first, then alphabetically
        entries.sort((a, b) => {
            if (a.isDirectory && !b.isDirectory) return -1;
            if (!a.isDirectory && b.isDirectory) return 1;
            return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
        });

        return { ok: true, entries };
    }
};

// Map JS error to structured error response
function mapError(e) {
    return {
        name: e.name || 'Error',
        message: e.message || String(e)
    };
}

// Handle incoming messages
self.onmessage = async (event) => {
    const { id, op, args } = event.data;

    try {
        const handler = operations[op];
        if (!handler) {
            self.postMessage({
                id,
                ok: false,
                error: { name: 'InvalidOperation', message: `Unknown operation: ${op}` }
            });
            return;
        }

        const result = await handler(args);

        // For read operations, transfer the buffer for zero-copy
        if (op === 'read' && result.data) {
            self.postMessage({ id, ...result }, [result.data.buffer]);
        } else {
            self.postMessage({ id, ...result });
        }
    } catch (e) {
        self.postMessage({
            id,
            ok: false,
            error: mapError(e)
        });
    }
};

// Signal that worker is ready
self.postMessage({ ready: true });
