import type { AuthTokenRequest, AuthTokenResult, Options } from './options';
export declare class TokenAuthError extends Error {
    readonly code?: number;
    constructor(message: string, code?: number);
}
export declare class TokenExpiredError extends TokenAuthError {
    constructor(message?: string);
}
export declare class TokenRevokedError extends TokenAuthError {
    constructor(message?: string);
}
export interface NormalizedAuthToken {
    token: string;
    issuedAt?: number;
    expiresAt?: number;
}
export declare function requestAuthToken(options: Pick<Options, 'authCallback' | 'authUrl'>, request: AuthTokenRequest): Promise<NormalizedAuthToken>;
export declare function normalizeAuthToken(result: AuthTokenResult): NormalizedAuthToken;
export declare function authTokenRefreshDelay(token: Pick<NormalizedAuthToken, 'issuedAt' | 'expiresAt'>, nowSeconds?: number): number | undefined;
