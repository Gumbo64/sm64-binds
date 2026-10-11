import sm64_ZIP_XOR from './sm64/pkg/sm64.us.wasm.zip.xor.png';
import JSZip from 'jszip';

import { xorBytes } from './xor.js';

// Function to unzip and return the file bytes
async function unzipBytes(zipBytes) {
    const jszip = new JSZip();
    try {
        const zip = await jszip.loadAsync(zipBytes);
        const file = Object.values(zip.files)[0];
        const fileBytes = await file.async('arraybuffer');
        return fileBytes;
    } catch (error) {
        console.error('Error unzipping file:', error);
        return null;
    }
}

async function calculateFileHash(arrayBuffer) {
    const hashBuffer = await crypto.subtle.digest('SHA-1', arrayBuffer);
    const hashArray = Array.from(new Uint8Array(hashBuffer));
    const hashHex = hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
    return hashHex;
};

async function storeROM(romArrayBuffer) {
    const romBuffer = new Uint8Array(romArrayBuffer);
    await storeFile("ROM", romBuffer);
    await updateWASMs();
}

async function getROM() {
    return await getFile("ROM");
}
async function getSM64WASM() {
    return await getFile("sm64.us.wasm");
}

async function isRomCached() {
    const request = await getROM();
    return request !== undefined;
}
async function updateWASMs() {
    await xorForWASMSingle(sm64_ZIP_XOR, "sm64.us.wasm");
    // await xorForWASMSingle(sm64_HEADLESSXOR, "sm64_headless.us.wasm");
}

async function xorForWASMSingle(wasmXOR, filename) {
    // Read and XOR the input file
    const key = await getFile("ROM");
    const inputFileResponse = await fetch(wasmXOR, { credentials: 'same-origin' });

    if (!inputFileResponse.ok) {
        throw new Error(`HTTP error! Status: ${inputFileResponse.status}`);
    }

    const keyBuffer = new Uint8Array(key);
    const inputFileBuffer = await inputFileResponse.arrayBuffer();
    const inputFileData = new Uint8Array(inputFileBuffer);

    const unzippedBytes = await unzipBytes(xorBytes(inputFileData, keyBuffer));

    // Initialize IndexedDB
    return storeFile(filename, unzippedBytes);
};

async function storeFile(filename, file) {
    // Initialize IndexedDB
    const db = await openDatabase();
    const transaction = db.transaction('files', 'readwrite');
    const store = transaction.objectStore('files');
    const request = store.put({ id: filename, data: file });

    return new Promise((resolve, reject) => {
        request.onsuccess = () => {
            resolve();
        };
        request.onerror = () => {
            reject(new Error(`Error saving '${filename}' to IndexedDB`));
        };
    });
}

async function getFile(filename) {
    const db = await openDatabase();
    const transaction = db.transaction('files', 'readonly');
    const store = transaction.objectStore('files');
    const request = store.get(filename);

    return new Promise((resolve, reject) => {
        request.onsuccess = (event) => {
            const result = event.target.result;
            if (result) {
                // Create a Blob from the stored data
                const blob = new Blob([result.data], { type: 'application/octet-stream' });
                
                // Read the Blob as an ArrayBuffer
                const reader = new FileReader();
                reader.onload = () => {
                    resolve(reader.result); // Resolve with the ArrayBuffer
                };
                reader.onerror = () => {
                    reject('Error reading the WASM Blob');
                };

                reader.readAsArrayBuffer(blob); // Convert Blob to ArrayBuffer
            } else {
                reject('File not found in IndexedDB');
            }
        };

        request.onerror = () => {
            reject('Error retrieving file from IndexedDB: ' + request.error);
        };
    });
}

// This function opens the IndexedDB database to be used in locateFile function
async function openDatabase() {
    return new Promise((resolve, reject) => {
        const request = indexedDB.open('SM64_LIB_WASM_DB', 1);

        request.onupgradeneeded = (event) => {
            const db = event.target.result;
            db.createObjectStore('files', { keyPath: 'id' });
            // resolve(db);
        };
        request.onsuccess = (event) => {
            resolve(event.target.result);
        };
        request.onerror = (event) => {
            reject(event.target.error);
        };
    });
}


export {calculateFileHash, storeROM, updateWASMs, isRomCached, getROM, getSM64WASM};