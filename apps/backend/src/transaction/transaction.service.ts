import { Injectable, Logger } from '@nestjs/common';
import { ConfigService } from '@nestjs/config';
import {
  TransactionDto,
  TransactionType,
  TransactionStatus,
} from './dto/transaction.dto';
import { getMockTransactions } from './mocks/mock-transactions';

interface HorizonOperation {
  id: string;
  type: string;
  created_at: string;
  transaction_hash: string;
  source_account: string;
  from?: string;
  to?: string;
  into?: string;
  amount?: string;
  amount_charged?: string;
  asset_type?: string;
  asset_code?: string;
  asset_issuer?: string;
  [key: string]: unknown;
}

interface HorizonResponse {
  _embedded: {
    records: HorizonTransaction[];
  };
  _links: {
    next?: {
      href: string;
    };
  };
}

interface HorizonErrorResponse {
  detail?: string;
  title?: string;
  status?: number;
}

interface HorizonTransaction {
  id: string;
  created_at: string;
  successful: boolean;
  memo?: string;
  fee_charged?: string;
}

interface OperationsResponse {
  _embedded?: {
    records: HorizonOperation[];
  };
}

@Injectable()
export class TransactionService {
  private readonly logger = new Logger(TransactionService.name);
  private readonly horizonUrl: string;
  private readonly useMockData: boolean;

  constructor(private configService: ConfigService) {
    const network = this.configService.get('STELLAR_NETWORK', 'testnet');
    this.horizonUrl =
      network === 'testnet'
        ? 'https://horizon-testnet.stellar.org'
        : 'https://horizon.stellar.org';

    this.useMockData =
      this.configService.get('USE_MOCK_TRANSACTIONS', 'true') === 'true';
    if (this.useMockData) {
      this.logger.log('Using mock transaction data for testing');
    }
  }

  async getTransactionHistory(
    publicKey: string,
    limit: number = 50,
    cursor?: string,
  ): Promise<{ transactions: TransactionDto[]; nextPage?: string }> {
    this.logger.log(`Fetching transaction history for ${publicKey}`);

    if (this.useMockData) {
      this.logger.log('Returning mock transaction data');
      return getMockTransactions(limit, cursor);
    }

    try {
      let url = `${this.horizonUrl}/accounts/${publicKey}/transactions?order=desc&limit=${limit}`;
      if (cursor) {
        url += `&cursor=${cursor}`;
      }

      const response = await fetch(url);
      const data = (await response.json()) as
        | HorizonResponse
        | HorizonErrorResponse;

      if (!response.ok) {
        const errorDetail = (data as HorizonErrorResponse).detail;
        const errorMessage = errorDetail || 'Failed to fetch transactions';
        throw new Error(errorMessage);
      }

      const horizonData = data as HorizonResponse;
      const transactions = await this.processTransactions(
        horizonData._embedded.records,
      );
      let nextPage: string | undefined;

      if (horizonData._links?.next?.href) {
        const nextUrl = new URL(horizonData._links.next.href);
        nextPage = nextUrl.searchParams.get('cursor') || undefined;
      }

      return { transactions, nextPage };
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : 'Unknown error';
      this.logger.error(`Failed to fetch transactions: ${errorMessage}`);
      return { transactions: [] };
    }
  }

  private async processTransactions(
    records: HorizonTransaction[],
  ): Promise<TransactionDto[]> {
    const transactions: TransactionDto[] = [];

    for (const record of records) {
      const operations = await this.getTransactionOperations(record.id);

      for (const operation of operations) {
        const transaction = this.mapToTransactionDto(operation, record);
        if (transaction) {
          transactions.push(transaction);
        }
      }
    }

    return transactions;
  }

  private async getTransactionOperations(
    transactionId: string,
  ): Promise<HorizonOperation[]> {
    try {
      const url = `${this.horizonUrl}/transactions/${transactionId}/operations`;
      const response = await fetch(url);
      const data = (await response.json()) as OperationsResponse;
      return data._embedded?.records || [];
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : 'Unknown error';
      this.logger.error(
        `Failed to fetch operations for ${transactionId}: ${errorMessage}`,
      );
      return [];
    }
  }

  private mapToTransactionDto(
    operation: HorizonOperation,
    transaction: HorizonTransaction,
  ): TransactionDto | null {
    const type = this.mapTransactionType(operation.type);
    if (!type) return null;

    const dto: TransactionDto = {
      id: operation.id,
      type,
      amount: this.getAmountFromOperation(operation),
      assetCode: this.getAssetCode(operation),
      assetIssuer: this.getAssetIssuer(operation),
      from: operation.source_account || operation.from || '',
      to: operation.to || operation.into || '',
      date: operation.created_at,
      status: transaction.successful
        ? TransactionStatus.SUCCESS
        : TransactionStatus.FAILED,
      transactionHash: transaction.id,
      memo: transaction.memo,
      fee: transaction.fee_charged,
    };

    return dto;
  }

  private mapTransactionType(horizonType: string): TransactionType | null {
    switch (horizonType) {
      case 'payment':
      case 'path_payment':
      case 'path_payment_strict_send':
      case 'path_payment_strict_receive':
        return TransactionType.PAYMENT;
      case 'manage_offer':
      case 'create_passive_offer':
        return TransactionType.SWAP;
      case 'change_trust':
        return TransactionType.TRUSTLINE;
      case 'create_account':
        return TransactionType.CREATE_ACCOUNT;
      case 'account_merge':
        return TransactionType.ACCOUNT_MERGE;
      default:
        return null;
    }
  }

  private getAmountFromOperation(operation: HorizonOperation): string {
    const amount = operation.amount ?? operation.amount_charged;
    return amount ?? '0';
  }

  private getAssetCode(operation: HorizonOperation): string {
    if (operation.asset_type === 'native') return 'XLM';
    const assetCode = operation.asset_code;
    return assetCode ?? (operation.asset_issuer ? 'Custom' : 'XLM');
  }

  private getAssetIssuer(operation: HorizonOperation): string | null {
    return operation.asset_issuer ?? null;
  }
}
