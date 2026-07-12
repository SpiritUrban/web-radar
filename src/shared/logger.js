export function createLogger(scope = 'app') {
  return {
    info(message, ...args) {
      console.log(`[${scope}] ${message}`, ...args);
    },
    error(message, ...args) {
      console.error(`[${scope}] ${message}`, ...args);
    }
  };
}
